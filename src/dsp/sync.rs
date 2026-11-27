use std::collections::VecDeque;

use crate::dsp::{DecodedFrame, DemodSample};

const SYNC_BITS: usize = 16;
const MIN_PREAMBLE_SAMPLE_ACCURACY_PER_SYMBOL: f32 = 0.70;

#[derive(Clone, Copy)]
struct PreambleCandidate {
    start_sample: u64,
    dc_offset: f32,
    sample_error_rate: f32,
    total_slope: f32,
}

enum SyncState {
    Searching {
        next_start: u64,
    },
    CollectingCandidates {
        next_start: u64,
        window_end: u64,
        candidates: Vec<PreambleCandidate>,
    },
    ConfirmingCandidate {
        candidate: PreambleCandidate,
        resume_at: u64,
    },
    ReadingLength {
        candidate: PreambleCandidate,
        next_symbol: u64,
    },
    ReadingFrame {
        candidate: PreambleCandidate,
        next_symbol: u64,
        remaining: usize,
        bytes: Vec<u8>,
    },
}

enum Availability<T> {
    Ready(T),
    NeedMoreSamples,
}

#[derive(Default)]
struct SampleBuffer {
    samples: VecDeque<DemodSample>,
    base_index: u64,
    end_index: u64,
}

impl SampleBuffer {
    fn extend(&mut self, samples: &[DemodSample]) {
        self.samples.extend(samples.iter().cloned());
        self.end_index = self
            .end_index
            .checked_add(samples.len() as u64)
            .expect("demodulated sample index overflow");
    }

    fn discriminator_at(&self, index: u64) -> Option<f32> {
        if index < self.base_index || index >= self.end_index {
            return None;
        }

        let index = usize::try_from(index - self.base_index).ok()?;
        Some(self.samples.get(index)?.discriminator)
    }

    fn sample_error_counts(
        &self,
        start: u64,
        end: u64,
        dc_offset: f32,
        expected_bit: u8,
    ) -> Option<(u64, u64)> {
        if start >= end || start < self.base_index || end > self.end_index {
            return None;
        }

        let start = usize::try_from(start - self.base_index).ok()?;
        let end = usize::try_from(end - self.base_index).ok()?;
        let error_count = self
            .samples
            .range(start..end)
            .filter(|sample| {
                let adjusted = sample.discriminator - dc_offset;
                adjusted == 0.0 || ((adjusted > 0.0) as u8) != expected_bit
            })
            .count();

        Some((error_count as u64, (end - start) as u64))
    }

    fn majority_bit(&self, start: u64, end: u64, dc_offset: f32) -> Option<u8> {
        if start >= end || start < self.base_index || end > self.end_index {
            return None;
        }

        let start = usize::try_from(start - self.base_index).ok()?;
        let end = usize::try_from(end - self.base_index).ok()?;
        let mut above_zero = 0;
        let mut below_zero = 0;

        for sample in self.samples.range(start..end) {
            let adjusted = sample.discriminator - dc_offset;
            above_zero += usize::from(adjusted > 0.0);
            below_zero += usize::from(adjusted < 0.0);
        }

        Some(u8::from(above_zero > below_zero))
    }

    fn power_sum(&self, start: u64, end: u64) -> Option<f64> {
        if start >= end || start < self.base_index || end > self.end_index {
            return None;
        }

        let start = usize::try_from(start - self.base_index).ok()?;
        let end = usize::try_from(end - self.base_index).ok()?;
        Some(
            self.samples
                .range(start..end)
                .map(|sample| f64::from(sample.iq.norm_sqr()))
                .sum(),
        )
    }

    fn discard_before(&mut self, index: u64) {
        let discard_to = index.min(self.end_index);
        let count = usize::try_from(discard_to.saturating_sub(self.base_index))
            .expect("discarded sample count does not fit in memory");

        for _ in 0..count {
            self.samples.pop_front();
        }
        self.base_index = discard_to;
    }
}

#[derive(Default)]
struct PowerAccumulator {
    power_sum: f64,
    sample_count: u64,
}

impl PowerAccumulator {
    fn add(&mut self, power_sum: f64, sample_count: u64) {
        self.power_sum += power_sum;
        self.sample_count += 1;
    }

    fn average(&self) -> Option<f64> {
        (self.sample_count > 0).then(|| self.power_sum / self.sample_count as f64)
    }
}

pub struct Sync {
    channel: u32,
    sample_rate: u32,
    symbol_rate: u32,
    preamble_bits: Vec<u8>,
    sync_word: u16,
    samples: SampleBuffer,
    state: SyncState,
    noise_power: PowerAccumulator,
    signal_power: PowerAccumulator,
    signal_power_next_sample: u64,
}

impl Sync {
    pub fn new(
        channel: u32,
        sample_rate: u32,
        symbol_rate: u32,
        preamble: u16,
        sync_word: u16,
    ) -> Self {
        assert!(sample_rate > 0, "Sample rate must be greater than zero");
        assert!(symbol_rate > 0, "Symbol rate must be greater than zero");
        assert!(
            sample_rate >= symbol_rate,
            "Sample rate must provide at least one sample per symbol"
        );
        let preamble_bits: Vec<u8> = (0..u16::BITS)
            .rev()
            .map(|bit| ((preamble >> bit) & 1) as u8)
            .collect();
        assert!(
            preamble_bits.contains(&0) && preamble_bits.contains(&1),
            "Preamble must contain both symbol values to estimate DC offset"
        );

        log::info!(
            "[{}] Synchronizer: {} preamble bits, {} sync bits, {} samples/symbol",
            channel,
            preamble_bits.len(),
            SYNC_BITS,
            sample_rate as f64 / symbol_rate as f64
        );

        Self {
            channel,
            sample_rate,
            symbol_rate,
            preamble_bits,
            sync_word,
            samples: SampleBuffer::default(),
            state: SyncState::Searching { next_start: 0 },
            noise_power: PowerAccumulator::default(),
            signal_power: PowerAccumulator::default(),
            signal_power_next_sample: 0,
        }
    }

    pub fn update(&mut self, demod_block: &[DemodSample]) -> Vec<DecodedFrame> {
        self.samples.extend(demod_block);
        self.process()
    }

    fn accumulate_signal_power_until(&mut self, end_sample: u64) {
        if end_sample == self.signal_power_next_sample {
            return;
        }

        let sample_count = end_sample
            .checked_sub(self.signal_power_next_sample)
            .expect("signal power sample index moved backwards");
        let power_sum = self
            .samples
            .power_sum(self.signal_power_next_sample, end_sample)
            .expect("signal power range must remain in the sample buffer");
        self.signal_power.add(power_sum, sample_count);
        self.signal_power_next_sample = end_sample;
    }

    fn accumulate_noise_power_until(&mut self, end_sample: u64) {
        let start_sample = self.samples.base_index;
        if end_sample <= start_sample {
            return;
        }

        let sample_count = end_sample - start_sample;
        let power_sum = self
            .samples
            .power_sum(start_sample, end_sample)
            .expect("search noise range must remain in the sample buffer");
        self.noise_power.add(power_sum, sample_count);
    }

    fn symbol_boundary(&self, origin: u64, symbol_index: u64) -> Option<u64> {
        let numerator = u128::from(symbol_index) * u128::from(self.sample_rate);
        let offset = numerator / u128::from(self.symbol_rate);
        origin.checked_add(u64::try_from(offset).ok()?)
    }

    fn symbol_range(&self, origin: u64, symbol_index: u64) -> Option<(u64, u64)> {
        let next_symbol = symbol_index.checked_add(1)?;
        Some((
            self.symbol_boundary(origin, symbol_index)?,
            self.symbol_boundary(origin, next_symbol)?,
        ))
    }

    fn symbol_bit(&self, origin: u64, symbol_index: u64, dc_offset: f32) -> Option<u8> {
        let (start, end) = self.symbol_range(origin, symbol_index)?;
        self.samples.majority_bit(start, end, dc_offset)
    }

    fn detect_preamble(&self, start_sample: u64) -> Availability<Option<PreambleCandidate>> {
        let Some(window_end) = self.symbol_boundary(start_sample, self.preamble_bits.len() as u64)
        else {
            return Availability::NeedMoreSamples;
        };
        if window_end > self.samples.end_index {
            return Availability::NeedMoreSamples;
        }

        let mut sample_distance = 0_u64;
        let mut total_samples = 0_u64;
        for (symbol_index, expected_bit) in self.preamble_bits.iter().enumerate() {
            let Some((symbol_start, symbol_end)) =
                self.symbol_range(start_sample, symbol_index as u64)
            else {
                return Availability::NeedMoreSamples;
            };
            let Some((sample_errors, symbol_samples)) =
                self.samples
                    .sample_error_counts(symbol_start, symbol_end, 0.0, *expected_bit)
            else {
                return Availability::NeedMoreSamples;
            };
            let sample_accuracy = (symbol_samples - sample_errors) as f32 / symbol_samples as f32;

            if sample_accuracy < MIN_PREAMBLE_SAMPLE_ACCURACY_PER_SYMBOL {
                return Availability::Ready(None);
            }

            let Some(bit) = self.samples.majority_bit(symbol_start, symbol_end, 0.0) else {
                return Availability::NeedMoreSamples;
            };
            if bit != *expected_bit {
                return Availability::Ready(None);
            }

            sample_distance += sample_errors;
            total_samples += symbol_samples;
        }

        let (dc_offset, total_slope) = match self.estimate_preamble_dc_offset(start_sample) {
            Availability::Ready(offset_and_slope) => offset_and_slope,
            Availability::NeedMoreSamples => return Availability::NeedMoreSamples,
        };

        let sample_error_rate = sample_distance as f32 / total_samples as f32;
        let candidate = PreambleCandidate {
            start_sample,
            dc_offset,
            sample_error_rate,
            total_slope,
        };

        log::debug!(
            "[{}] preamble candidate: sample window [{}..{}), dc_offset={:+.6}, total_slope={:.6}, sample_distance={}/{}, sample_error_rate={:.5}",
            self.channel,
            candidate.start_sample,
            window_end,
            candidate.dc_offset,
            candidate.total_slope,
            sample_distance,
            total_samples,
            candidate.sample_error_rate
        );

        Availability::Ready(Some(candidate))
    }

    fn estimate_preamble_dc_offset(&self, start_sample: u64) -> Availability<(f32, f32)> {
        let mut edge_sums = [0.0_f32; 2];
        let mut edge_counts = [0_u32; 2];
        let mut slope_sums = [0.0_f32; 2];

        for (symbol_index, bit) in self.preamble_bits.iter().enumerate() {
            let Some((start, end)) = self.symbol_range(start_sample, symbol_index as u64) else {
                return Availability::NeedMoreSamples;
            };
            let Some(entry) = self.samples.discriminator_at(start) else {
                return Availability::NeedMoreSamples;
            };
            let Some(exit) = self.samples.discriminator_at(end - 1) else {
                return Availability::NeedMoreSamples;
            };

            let bit_index = usize::from(*bit);
            edge_sums[bit_index] += entry + exit;
            edge_counts[bit_index] += 2;
            slope_sums[bit_index] += exit - entry;
        }

        let symbol_0_average = edge_sums[0] / edge_counts[0] as f32;
        let symbol_1_average = edge_sums[1] / edge_counts[1] as f32;
        let dc_offset = (symbol_0_average + symbol_1_average) / 2.0;
        let total_slope = slope_sums[0].abs() + slope_sums[1].abs();
        Availability::Ready((dc_offset, total_slope))
    }

    fn candidate_window_end(&self, first_start: u64) -> u64 {
        let phase_count = u64::from(self.sample_rate).div_ceil(u64::from(self.symbol_rate));
        first_start
            .checked_add(phase_count.saturating_sub(1))
            .expect("demodulated sample index overflow")
    }

    fn rank_candidates_by_slope(candidates: &mut [PreambleCandidate]) {
        candidates.sort_by(|left, right| {
            left.total_slope
                .total_cmp(&right.total_slope)
                .then_with(|| left.sample_error_rate.total_cmp(&right.sample_error_rate))
                .then_with(|| left.start_sample.cmp(&right.start_sample))
        });
    }

    fn confirm_sync(&self, candidate: PreambleCandidate) -> Availability<bool> {
        let first_symbol = self.preamble_bits.len() as u64;
        let Some((high_byte, next_symbol)) = self.decode_byte(candidate, first_symbol) else {
            return Availability::NeedMoreSamples;
        };
        let Some((low_byte, _)) = self.decode_byte(candidate, next_symbol) else {
            return Availability::NeedMoreSamples;
        };
        let detected_sync = u16::from_be_bytes([high_byte, low_byte]);

        if detected_sync != self.sync_word {
            log::debug!(
                "[{}]  mismatched sync word {:04X} instead of {:04X}",
                self.channel,
                detected_sync,
                self.sync_word
            );
        }

        Availability::Ready(detected_sync == self.sync_word)
    }

    fn decode_byte(&self, candidate: PreambleCandidate, first_symbol: u64) -> Option<(u8, u64)> {
        let mut byte = 0_u8;
        for bit_index in 0..8_u64 {
            let bit = self.symbol_bit(
                candidate.start_sample,
                first_symbol + bit_index,
                candidate.dc_offset,
            )?;
            byte = (byte << 1) | bit;
        }
        Some((byte, first_symbol + 8))
    }

    fn check_crc(&self, data: &[u8]) -> bool {
        if data.len() < 3 {
            log::warn!(
                "[{}] cannot check CRC: frame has {} bytes, expected length byte and 2 CRC bytes",
                self.channel,
                data.len()
            );
            return false;
        }

        let calculated = crc16::State::<crc16::EN_13757>::calculate(&data[..data.len() - 2]);
        let received = (u16::from(data[data.len() - 2]) << 8) | u16::from(data[data.len() - 1]);
        let valid = calculated == received;

        log::debug!(
            "[{}] CRC16 EN-13757: calculated={:04X}, received={:04X}, valid={}",
            self.channel,
            calculated,
            received,
            valid
        );

        valid
    }

    fn process(&mut self) -> Vec<DecodedFrame> {
        let mut frames = Vec::new();

        loop {
            let state = std::mem::replace(&mut self.state, SyncState::Searching {next_start: self.samples.end_index});

            match state {

                SyncState::Searching{ next_start } => match self.detect_preamble(next_start) {
                    Availability::NeedMoreSamples => {
                        self.state = SyncState::Searching { next_start };
                        break;
                    }
                    Availability::Ready(Some(candidate)) => {
                        let window_end = self.candidate_window_end(candidate.start_sample);
                        let next_start = candidate.start_sample + 1;
                        self.state = SyncState::CollectingCandidates {next_start, window_end, candidates: vec![candidate]};
                    }
                    Availability::Ready(None) => {
                        let next_start = next_start + 1;
                        self.accumulate_noise_power_until(next_start);
                        self.samples.discard_before(next_start);
                        self.state = SyncState::Searching { next_start };
                    }
                },
                SyncState::CollectingCandidates {next_start, window_end, mut candidates} => {
                    if next_start > window_end {
                        let resume_at = candidates
                            .iter()
                            .map(|candidate| candidate.start_sample)
                            .max()
                            .expect("candidate collection must not be empty")
                            .checked_add(1)
                            .expect("demodulated sample index overflow");
                        Self::rank_candidates_by_slope(&mut candidates);
                        let slope_candidate = candidates[0];
                        log::debug!("[{}] checking {} preamble candidates ranked by total_slope; selected sample {}", self.channel, candidates.len(), slope_candidate.start_sample);
                        self.state = SyncState::ConfirmingCandidate{candidate: slope_candidate, resume_at};
                        continue;
                    }

                    match self.detect_preamble(next_start) {
                        Availability::NeedMoreSamples => {
                            self.state = SyncState::CollectingCandidates{next_start, window_end, candidates};
                            break;
                        }
                        Availability::Ready(Some(candidate)) => candidates.push(candidate),
                        Availability::Ready(None) => {}
                    }

                    self.state = SyncState::CollectingCandidates {next_start: next_start + 1, window_end, candidates};
                }
                SyncState::ConfirmingCandidate{candidate, resume_at} => match self.confirm_sync(candidate) {
                    Availability::NeedMoreSamples => {
                        self.state = SyncState::ConfirmingCandidate {candidate, resume_at};
                        break;
                    }
                    Availability::Ready(false) => {
                        log::debug!("[{}] sync mismatch at selected preamble candidate sample {}; resuming search at sample {}", self.channel, candidate.start_sample, resume_at);
                        self.samples.discard_before(resume_at);
                        self.state = SyncState::Searching{next_start: resume_at};
                    }
                    Availability::Ready(true) => {
                        log::debug!("[{}] sync word confirmed at candidate sample {}", self.channel, candidate.start_sample);
                        let next_symbol = (self.preamble_bits.len() + SYNC_BITS) as u64;
                        let next_sample = self.symbol_boundary(candidate.start_sample, next_symbol).expect("frame sample index overflow");
                        self.signal_power = PowerAccumulator::default();
                        self.signal_power_next_sample = candidate.start_sample;
                        self.accumulate_signal_power_until(next_sample);
                        self.samples.discard_before(next_sample);
                        self.state = SyncState::ReadingLength{candidate, next_symbol};
                    }
                },
                SyncState::ReadingLength{candidate, next_symbol} => {
                    let Some((length, next_symbol)) = self.decode_byte(candidate, next_symbol)
                    else {
                        self.state = SyncState::ReadingLength {candidate, next_symbol};
                        break;
                    };

                    let remaining = usize::from(length);
                    let next_sample = self
                        .symbol_boundary(candidate.start_sample, next_symbol)
                        .expect("frame sample index overflow");
                    self.accumulate_signal_power_until(next_sample);
                    self.samples.discard_before(next_sample);

                    if remaining == 0 {
                        log::warn!("[{}] ignoring empty frame: length field leaves no bytes for CRC", self.channel);
                        self.state = SyncState::Searching {next_start: next_sample};
                    } else {
                        self.state = SyncState::ReadingFrame {candidate, next_symbol, remaining, bytes: vec![length]};
                    }
                }
                SyncState::ReadingFrame {candidate, next_symbol, remaining, mut bytes} => {
                    let Some((byte, next_symbol)) = self.decode_byte(candidate, next_symbol) else {
                        self.state = SyncState::ReadingFrame {candidate, next_symbol, remaining, bytes};
                        break;
                    };

                    bytes.push(byte);
                    let remaining = remaining - 1;
                    let next_sample = self
                        .symbol_boundary(candidate.start_sample, next_symbol)
                        .expect("frame sample index overflow");
                    self.accumulate_signal_power_until(next_sample);
                    self.samples.discard_before(next_sample);

                    if remaining == 0 {
                        let crc_valid = self.check_crc(&bytes);
                        let signal_power = self
                            .signal_power
                            .average()
                            .expect("decoded frame must contain signal samples");
                        let noise_power = self.noise_power.average();
                        let snr_db = noise_power
                            .filter(|power| *power > 0.0)
                            .map(|power| 10.0 * (signal_power / power).log10());
                        let crc_status = if crc_valid {
                            "\x1b[32mvalid\x1b[0m"
                        } else {
                            "\x1b[31minvalid\x1b[0m"
                        };
                        log::info!(
                            "[{}] decoded {}-byte frame (CRC: {}, noise power: {:?}, signal power: {:.6}, SNR: {:?} dB)",
                            self.channel,
                            bytes.len(),
                            crc_status,
                            noise_power,
                            signal_power,
                            snr_db
                        );
                        frames.push(DecodedFrame {
                            channel: self.channel,
                            bytes,
                            crc_valid,
                            sample_error_rate: candidate.sample_error_rate,
                            noise_power,
                            signal_power,
                            snr_db,
                        });
                        self.state = SyncState::Searching {
                            next_start: next_sample,
                        };
                    } else {
                        self.state = SyncState::ReadingFrame {
                            candidate,
                            next_symbol,
                            remaining,
                            bytes,
                        };
                    }
                }
            }
        }

        frames
    }
}

#[cfg(test)]
mod tests {
    use num_complex::Complex32;

    use super::{Availability, PreambleCandidate, SYNC_BITS, Sync};
    use crate::dsp::DemodSample;

    const PREAMBLE: u16 = 0x5555;
    const PREAMBLE_BITS: usize = u16::BITS as usize;
    const SYNC_WORD: u16 = 0xF672;

    fn bit_sequence(value: u64, bit_count: usize) -> Vec<u8> {
        (0..bit_count)
            .rev()
            .map(|bit| ((value >> bit) & 1) as u8)
            .collect()
    }

    fn preamble_and_sync_bits() -> Vec<u8> {
        let mut bits = bit_sequence(u64::from(PREAMBLE), PREAMBLE_BITS);
        bits.extend(bit_sequence(u64::from(SYNC_WORD), SYNC_BITS));
        bits
    }

    fn frame_with_crc(payload: &[u8]) -> Vec<u8> {
        let following_len = payload.len() + 2;
        let mut frame = Vec::with_capacity(following_len + 1);
        frame.push(u8::try_from(following_len).unwrap());
        frame.extend_from_slice(payload);
        let crc = crc16::State::<crc16::EN_13757>::calculate(&frame);
        frame.extend_from_slice(&crc.to_be_bytes());
        frame
    }

    fn sample(discriminator: f32) -> DemodSample {
        DemodSample {
            iq: Complex32::new(discriminator, 0.0),
            discriminator,
        }
    }

    fn encoded_samples(
        sample_rate: u32,
        symbol_rate: u32,
        prefix_samples: usize,
        packet_bytes: &[u8],
    ) -> Vec<DemodSample> {
        let mut bits = preamble_and_sync_bits();
        for byte in packet_bytes {
            bits.extend(bit_sequence(u64::from(*byte), 8));
        }

        let packet_samples =
            (bits.len() as u64 * u64::from(sample_rate)).div_ceil(u64::from(symbol_rate));
        let mut samples = vec![sample(1.0); prefix_samples + packet_samples as usize];

        for (symbol_index, bit) in bits.iter().enumerate() {
            let start = prefix_samples as u64
                + (symbol_index as u64 * u64::from(sample_rate)) / u64::from(symbol_rate);
            let end = prefix_samples as u64
                + ((symbol_index as u64 + 1) * u64::from(sample_rate)) / u64::from(symbol_rate);
            let discriminator = if *bit == 1 { 1.0 } else { -1.0 };
            for sample_index in start..end {
                samples[sample_index as usize] = sample(discriminator);
            }
        }

        samples
    }

    fn preamble_with_first_symbol_accuracy(correct_samples: usize) -> Vec<DemodSample> {
        let bits = bit_sequence(u64::from(PREAMBLE), PREAMBLE_BITS);
        let mut samples = Vec::new();

        for (symbol_index, expected_bit) in bits[..16].iter().enumerate() {
            for sample_index in 0..20 {
                let value = if symbol_index == 0 && sample_index >= correct_samples {
                    0.0
                } else {
                    if *expected_bit == 1 { 1.0 } else { -1.0 }
                };
                samples.push(sample(value));
            }
        }

        samples
    }

    fn decode_in_chunks(samples: &[DemodSample], chunk_size: usize) -> Vec<Vec<u8>> {
        let mut decoder = Sync::new(0, 10, 3, PREAMBLE, SYNC_WORD);
        let mut frames = Vec::new();
        for chunk in samples.chunks(chunk_size) {
            frames.extend(decoder.update(chunk).into_iter().map(|frame| frame.bytes));
        }
        frames
    }

    #[test]
    fn candidate_order_uses_slope_not_sample_error_rate() {
        let mut candidates: Vec<PreambleCandidate> = (10..15)
            .zip([
                (0.01, 5.0),
                (0.02, 4.0),
                (0.03, 3.0),
                (0.04, 1.0),
                (0.05, 2.0),
            ])
            .map(
                |(start_sample, (sample_error_rate, total_slope))| PreambleCandidate {
                    start_sample,
                    dc_offset: 0.0,
                    sample_error_rate,
                    total_slope,
                },
            )
            .collect();

        Sync::rank_candidates_by_slope(&mut candidates);

        assert_eq!(candidates[0].start_sample, 13);
        assert_eq!(candidates[0].total_slope, 1.0);
        assert_eq!(candidates[0].sample_error_rate, 0.04);
    }

    #[test]
    fn candidate_order_uses_sample_error_rate_to_break_slope_ties() {
        let mut candidates: Vec<PreambleCandidate> = (10..15)
            .map(|start_sample| PreambleCandidate {
                start_sample,
                dc_offset: 0.0,
                sample_error_rate: if start_sample == 10 { 0.01 } else { 0.02 },
                total_slope: 0.0,
            })
            .collect();

        Sync::rank_candidates_by_slope(&mut candidates);

        assert_eq!(candidates[0].start_sample, 10);
    }

    #[test]
    fn symbol_bit_uses_majority_instead_of_being_flipped_by_a_spike() {
        let mut decoder = Sync::new(0, 20, 1, PREAMBLE, SYNC_WORD);
        let mut samples = vec![sample(-1.0); 20];
        samples[0] = sample(100.0);
        decoder.samples.extend(&samples);

        assert!(
            samples
                .iter()
                .map(|sample| sample.discriminator)
                .sum::<f32>()
                > 0.0
        );
        assert_eq!(decoder.symbol_bit(0, 0, 0.0), Some(0));
    }

    #[test]
    fn estimates_dc_offset_from_preamble_symbol_edges() {
        let mut decoder = Sync::new(0, 10, 1, PREAMBLE, SYNC_WORD);
        let bits = bit_sequence(u64::from(PREAMBLE), PREAMBLE_BITS);
        let mut samples = Vec::new();

        for bit in &bits[..16] {
            let (entry_value, exit_value) = if *bit == 0 { (0.1, 0.3) } else { (0.5, 0.8) };
            let interior_value = if *bit == 0 { -0.7 } else { 2.3 };
            for sample_index in 0..10 {
                let value = if sample_index == 0 || sample_index == 9 {
                    if sample_index == 0 {
                        entry_value
                    } else {
                        exit_value
                    }
                } else {
                    interior_value
                };
                samples.push(sample(value));
            }
        }
        decoder.samples.extend(&samples);

        let Availability::Ready(Some(candidate)) = decoder.detect_preamble(0) else {
            panic!("expected the synthetic preamble to be detected");
        };
        assert!((candidate.dc_offset - 0.425).abs() < 0.0001);
        assert!((candidate.total_slope - 4.0).abs() < 0.0001);
    }

    #[test]
    fn preamble_requires_forty_percent_correct_samples_in_each_symbol() {
        let mut decoder = Sync::new(0, 20, 1, PREAMBLE, SYNC_WORD);
        decoder
            .samples
            .extend(&preamble_with_first_symbol_accuracy(7));
        assert!(matches!(
            decoder.detect_preamble(0),
            Availability::Ready(None)
        ));

        let mut decoder = Sync::new(0, 20, 1, PREAMBLE, SYNC_WORD);
        decoder
            .samples
            .extend(&preamble_with_first_symbol_accuracy(8));
        assert!(matches!(
            decoder.detect_preamble(0),
            Availability::Ready(Some(_))
        ));
    }

    #[test]
    fn preamble_rejects_any_symbol_with_a_wrong_majority_bit() {
        let mut decoder = Sync::new(0, 20, 1, PREAMBLE, SYNC_WORD);
        let bits = bit_sequence(u64::from(PREAMBLE), PREAMBLE_BITS);
        let mut samples = Vec::new();

        for (symbol_index, expected_bit) in bits[..16].iter().enumerate() {
            for sample_index in 0..20 {
                let bit = if symbol_index == 0 && sample_index >= 8 {
                    *expected_bit ^ 1
                } else {
                    *expected_bit
                };
                samples.push(sample(if bit == 1 { 1.0 } else { -1.0 }));
            }
        }
        decoder.samples.extend(&samples);

        assert!(matches!(
            decoder.detect_preamble(0),
            Availability::Ready(None)
        ));
    }

    #[test]
    fn decodes_fractional_samples_per_symbol_independent_of_chunking() {
        let expected = vec![frame_with_crc(&[0xA5, 0x5A])];
        let mut samples = encoded_samples(10, 3, 7, &expected[0]);
        for sample in &mut samples {
            sample.discriminator += 0.37;
        }

        for chunk_size in [1, 2, 7, 31, samples.len()] {
            assert_eq!(decode_in_chunks(&samples, chunk_size), expected);
        }
    }

    #[test]
    fn resumes_search_after_a_completed_frame() {
        let first = frame_with_crc(&[0xA5, 0x5A]);
        let second = frame_with_crc(&[0xC3]);
        let mut samples = encoded_samples(10, 3, 7, &first);
        samples.extend(encoded_samples(10, 3, 0, &second));

        assert_eq!(
            decode_in_chunks(&samples, samples.len()),
            vec![first, second]
        );
    }

    #[test]
    fn rejects_a_candidate_with_an_invalid_sync_word() {
        let packet = [2_u8, 0xA5, 0x5A];
        let mut bits = preamble_and_sync_bits();
        bits[PREAMBLE_BITS + SYNC_BITS - 3] ^= 1;
        bits[PREAMBLE_BITS + SYNC_BITS - 7] ^= 1;
        bits.extend(
            packet
                .iter()
                .flat_map(|byte| bit_sequence(u64::from(*byte), 8)),
        );

        let sample_rate = 10_u32;
        let symbol_rate = 3_u32;
        let prefix_samples = 7_u64;
        let packet_samples =
            (bits.len() as u64 * u64::from(sample_rate)).div_ceil(u64::from(symbol_rate));
        let mut samples = vec![sample(1.0); (prefix_samples + packet_samples) as usize];
        for (symbol_index, bit) in bits.iter().enumerate() {
            let start = prefix_samples
                + (symbol_index as u64 * u64::from(sample_rate)) / u64::from(symbol_rate);
            let end = prefix_samples
                + ((symbol_index as u64 + 1) * u64::from(sample_rate)) / u64::from(symbol_rate);
            let discriminator = if *bit == 1 { 1.0 } else { -1.0 };
            for sample_index in start..end {
                samples[sample_index as usize] = sample(discriminator);
            }
        }

        let mut decoder = Sync::new(0, sample_rate, symbol_rate, PREAMBLE, SYNC_WORD);
        assert!(decoder.update(&samples).is_empty());
    }

    #[test]
    fn checks_en_13757_crc_and_rejects_corrupted_frames() {
        let decoder = Sync::new(0, 10, 3, PREAMBLE, SYNC_WORD);
        let mut frame = frame_with_crc(&[0xA5, 0x5A]);

        assert!(decoder.check_crc(&frame));

        frame[1] ^= 1;
        assert!(!decoder.check_crc(&frame));
        assert!(!decoder.check_crc(&[2, 0]));
    }

    #[test]
    fn returns_power_metrics_for_frames_with_invalid_crc() {
        let mut frame = frame_with_crc(&[0xA5, 0x5A]);
        frame[1] ^= 1;
        let mut samples = encoded_samples(10, 3, 7, &frame);
        for sample in &mut samples[..7] {
            sample.iq = Complex32::new(0.5, 0.0);
        }
        for sample in &mut samples[7..] {
            sample.iq = Complex32::new(2.0, 0.0);
        }

        let mut decoder = Sync::new(4, 10, 3, PREAMBLE, SYNC_WORD);
        let decoded = decoder.update(&samples);
        assert_eq!(decoded.len(), 1);
        let decoded = &decoded[0];

        assert_eq!(decoded.channel, 4);
        assert_eq!(decoded.bytes, frame);
        assert!(!decoded.crc_valid);
        assert_eq!(decoded.noise_power, Some(0.25));
        assert!((decoded.signal_power - 4.0).abs() < 0.0001);
        assert!((decoded.snr_db.unwrap() - 10.0 * 16.0_f64.log10()).abs() < 0.0001);
    }

    #[test]
    fn decoded_frame_reports_valid_crc() {
        let frame = frame_with_crc(&[0xA5, 0x5A]);
        let samples = encoded_samples(10, 3, 7, &frame);
        let mut decoder = Sync::new(0, 10, 3, PREAMBLE, SYNC_WORD);

        let decoded = decoder.update(&samples);

        assert_eq!(decoded.len(), 1);
        assert!(decoded[0].crc_valid);
    }

    #[test]
    fn searching_keeps_only_the_unprocessed_preamble_window() {
        let mut decoder = Sync::new(0, 10, 3, PREAMBLE, SYNC_WORD);
        let noise = vec![sample(1.0); 1_000];
        decoder.update(&noise);

        assert!(decoder.samples.samples.len() <= 55);
    }
}
