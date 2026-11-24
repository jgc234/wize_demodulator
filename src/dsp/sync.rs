
use std::{collections::VecDeque};
use log;
use std::fmt;
use std::fmt::Write;

use crate::dsp::DemodSample;

//use crc16;
//use chrono;
//use crate::dsp::{FrameResult};
//use num_complex::Complex32;
//use std::fmt;
//use gethostname::gethostname;

#[derive(PartialEq)]
enum SyncState {
    Searching,
    Preamble(Vec<PreambleCandidate>),
    Sync(Vec<u8>),
    Frame(Vec<u8>),
}

#[derive(Copy,Clone,PartialEq)]
struct PreambleCandidate {
    buf_start: usize,
    buf_end: usize,
    dc_offset: f32,
    symbol_0_ramp: f32,
    symbol_1_ramp: f32,
    symbol_ramp: f32,
    hamming_distance: u32,
    hamming_rate: f32,
}

impl fmt::Debug for PreambleCandidate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PreambleCandidate {{ ubffer_offset: {}, dc_offset: {:+.4}, symbol_0_ramp: {:+.4}, symbol_1_ramp: {:+.4}, symbol_ramp: {:+.4}, hamming_distance: {:+.4}, hamming_rate: {:.4} }}",
            self.buffer_offset, self.dc_offset, self.symbol_0_ramp, self.symbol_1_ramp, self.symbol_avg_ramp, self.hamming_distance, self.hamming_rate
        )
    }
}


pub struct Sync {
    channel: u32,
    state: SyncState,
    preamble_vec: Vec<u8>,
    sync_vec: Vec<u8>,
    buffer: VecDeque<DemodSample>,
    buffer_offset: usize,
    samples_per_symbol: f32,
}


impl Sync {

    pub fn new(channel: u32, samples_per_symbol: f32) -> Self {

        // assumed preable is 0x5555.
        // Generate preamble with samples_per_symbol repetitions for each symbol
        // build a vector of sine wave that repreents the preamble.
        let preamble_size: usize = 16;
        let preamble_value = 0x5555_u16;
        let sync_value = 0xF672_u16;
        let sync_size: usize = 2;
        let mut preamble_vec = Vec::with_capacity(preamble_size);
        let mut sync_vec = vec![0xF6, 0x72];

        log::info!("Preamble for channel {}: size={} vec={:?}", channel, preamble_vec.len(), preamble_vec);
    
        Self {
            channel: channel,
            state: SyncState::Searching,
            preamble_vec: preamble_vec,
            sync_vec: sync_vec,
            buffer: VecDeque::new(),
            buffer_offset: 0,
            samples_per_symbol: samples_per_symbol,
        }
    }

    pub fn update(&mut self, demod_block: &[DemodSample]) {

        self.buffer.extend(demod_block.iter().cloned());
        self.process()

    }

    fn check_sync_candidate(&self) -> Option<PreambleCandidate> {

        // this is a more detailed check on a specific offset for a sliding window in the buffer.

        let mut symbol_0_hamming_distance = 0;
        let mut symbol_1_hamming_distance = 0;
        let mut symbol_0_leading_sum = 0.0;
        let mut symbol_1_leading_sum = 0.0;
        let mut symbol_0_trailing_sum = 0.0;
        let mut symbol_1_trailing_sum = 0.0;
        let mut preamble_hamming_distance = 0;

        // TODO - fix hardcoded preamble length.
        for n in 0..16 {

            let start = self.buffer_offset + (n as f32 * self.samples_per_symbol) as usize;
            let end = self.buffer_offset + ((n as f32 + 1.0) * self.samples_per_symbol) as usize;
            let expected_bit = n as u8 % 2;
            let mut symbol_hamming_distance = 0;
            let leading_value = self.buffer[start].discriminator;
            let trailing_value = self.buffer[end - 1].discriminator;
                            
            let mut output = String::new();

            // iterate samples over a single symbol.
            for j in start..end {
                let value = self.buffer[j].discriminator;
                let binary_value = (value >= 0.0) as u8;
                let d = (binary_value ^ expected_bit) as u32;
                symbol_hamming_distance += d;
                let ch = if binary_value == 0 { '.'} else { '█' };
                output.push(ch);
            }

            preamble_hamming_distance += symbol_hamming_distance;

            if expected_bit == 0 {
                symbol_0_hamming_distance += symbol_hamming_distance;
                symbol_0_leading_sum += leading_value;
                symbol_0_trailing_sum += trailing_value;
            } else {
                symbol_1_hamming_distance += symbol_hamming_distance;
                symbol_1_leading_sum += leading_value;
                symbol_1_trailing_sum += trailing_value;
            }

            log::debug!("     [{}] Symbol {:2}, symbol[{}] - ε{:2} ◀{:+.4} ▶{:+.4} samples: [{}]", 
                self.channel, n,
                expected_bit,
                symbol_hamming_distance,
                leading_value, trailing_value,
                output
            );
            // if the symbol distance is a fail, bailout and return.
            // 
            if symbol_hamming_distance as f32 / self.samples_per_symbol > 0.4 {
                log::debug!("     [{}] Symbol {:2} - failed per symbol hamming distance check", self.channel, n);
                return None;
            }

        }

        let symbol_0_leading_avg = symbol_0_leading_sum / 16.0;
        let symbol_0_trailing_avg = symbol_0_trailing_sum / 16.0;
        let symbol_0_ramp = symbol_0_trailing_avg - symbol_0_leading_avg;
        let symbol_1_leading_avg = symbol_1_leading_sum / 16.0;
        let symbol_1_trailing_avg = symbol_1_trailing_sum / 16.0;
        let symbol_1_ramp = symbol_1_trailing_avg - symbol_1_leading_avg;
        let symbol_0_dc_offset = (symbol_0_leading_avg + symbol_0_trailing_avg) / 2.0;
        let symbol_1_dc_offset = (symbol_1_leading_avg + symbol_1_trailing_avg) / 2.0;
        let leading_offset = (symbol_0_leading_avg + symbol_1_leading_avg) / 2.0;
        let trailing_offset = (symbol_0_trailing_avg + symbol_1_trailing_avg) / 2.0;
        let dc_offset = (symbol_0_dc_offset + symbol_1_dc_offset) / 2.0;
        let ramp_diff = symbol_0_ramp - symbol_1_ramp;
        let ramp_diff_abs = ramp_diff.abs();
        let hamming_rate = preamble_hamming_distance as f32 / 16 as f32; // TODO handcoded premable length.

        log::info!("    [{}] Preamble offset {:04} - symbol_0 - ε{:2} ◀{:+.4} ▶{:+.4} μ{:+.4}, ramp {:+.4}, symbol_1 - ε{:2} ◀{:+.4} ▶{:+.4} μ{:+.4} ramp {:+.4}, overall  - ε{:2} ◀{:+.4} ▶{:+.4} μ{:+.4} ramp Δ{:+.4} abs Δ{:+.4}",
            self.channel, offset,
            symbol_0_hamming_distance, symbol_0_leading_avg, symbol_0_trailing_avg, symbol_0_dc_offset, symbol_0_ramp,
            symbol_1_hamming_distance, symbol_1_leading_avg, symbol_1_trailing_avg, symbol_1_dc_offset, symbol_1_ramp,
            preamble_hamming_distance, leading_offset, trailing_offset, offset, ramp_diff, ramp_diff_abs
        );

        Some(PreambleCandidate{
            buf_offset_start: offset,
            buf_offset_end: offset + xx,
            dc_offset: dc_offset,
            symbol_0_ramp: symbol_0_ramp,
            symbol_1_ramp: symbol_1_ramp,
            symbol_ramp: ramp_diff_abs,
            hamming_distance: preamble_hamming_distance,
            hamming_rate: hamming_rate,
        })

    }


    // two options for processing:
    // - match on each state.. then process as many samples as possible under that state.
    // - or .. process each sample.. and swithch on the state.
    //


    pub fn process(&mut self) {

        // greedy states - each state can consume a number of samples until it 
        // yeilds or changes state.

        while self.buffer.len() > (self.buffer_offset + self.samples_per_symbol * 2) {

            match &mut self.state {
                SyncState::Searching => {
                    let result = self.check_sync_candidate();
                    match result {
                        Some(x) => {
                            self.state = SyncState::Preamble(vec![x])
                        },
                        None => {
                            // should drain buffer to this point.
                        }
                    }
                },
                SyncState::Preamble(x) => {
                    let result = self.check_sync_candidate();
                    match &mut result {
                        Some(x) => {
                            x.push(x);
                        },
                        None => {
                            let candidate = self.select_candidate(x)
                            if let Some(x) = candidate {
                                self.state = SyncState::Sync(Vec::new())

                            }
                        }                        
                    }
                },
                SyncState::Sync(x) => {
                    self.process_sync();
                },
                SyncState::Frame(x) => {
                    self.process_frame();
                },
            }
        }

        // look for preamble in demod_buffer

        if self.buffer.len() < self.preamble.len() + self.samples_per_symbol as usize {
            // not enough samples to process
            return;
        }

        for i in 0..(self.buffer.len() - self.preamble.len()) {

            if self.state == SyncState::Preamble  || self.state == SyncState::Searching {

                let result = self.check_sync_candidate(i);

                log::debug!(" - result {:?}", result);

                match result {
                    Some(candidate) => {
                        self.preamble_candidates.push(candidate);
                        self.state = SyncState::Preamble;
                    },
                    None => {
                        if self.state == SyncState::Preamble {
                            if let Some(selected) = self.select_candidate() {
                                log::info!("[{}] selected candidate - {:?}", self.channel, selected);
                            }
                            self.state = SyncState::Sync;
                            self.preamble_candidates.clear();
                        }
                        self.state = SyncState::Searching;
                    },
                }
            }

        }

        // remove all samples up to the last 20 samples to keep some overlap for next block
        let remove_count = self.buffer.len() - self.preamble.len();
        self.buffer.drain(0..remove_count);
        
    }

    fn select_candidate(&mut self) -> Option<(PreambleCandidate)> {
        return  self.preamble_candidates.iter().min_by(|a, b| a.symbol_avg_ramp.partial_cmp(&b.symbol_avg_ramp).unwrap()).copied();
    }

}