

use std::{collections::VecDeque};
use log;
use crc16;
use chrono;
use crate::dsp::{FrameResult};
use num_complex::Complex32;
use std::fmt;

#[derive(Copy, Clone)]
struct SyncMatch {
    match_index: usize,
    hamming_ratio: f32,
    signal_deviation: f32,
    power_sum: f32,
    power_count: usize,
    saved_floor_power: f32,
}


pub struct Sync {
    channel: u32,
    preamble: Vec<u8>,
    demod_buffer: VecDeque<f32>,
    binary_buffer: VecDeque<u8>,
    iq_buffer: VecDeque<Complex32>,
    power_buffer: VecDeque<f32>,
    hamming_threshold: f32,
    samples_per_symbol: usize,
    matchlist: Vec<SyncMatch>,
    bit_buffer: Vec<u8>,
    byte_buffer: Vec<u8>,
    sync_details: Option<SyncMatch>,
    sample_counter: u64,
    time_offset: f32,
    floor_power: f32,
    floor_alpha: f32,
}


impl SyncMatch {

    pub fn update_power(&mut self, iq_sample: Complex32) {
        self.power_sum += iq_sample.norm_sqr();
        self.power_count += 1;
    }

    pub fn calculate_final_power(&self) -> f32 {
        if self.power_count > 0 {
            10.0 * ( self.power_sum / self.power_count as f32).log10()
        } else {
            0.0
        }
    }

    pub fn calculate_snr(&self, n:f32) -> f32 {
        if self.power_count > 0 {
            let s = self.power_sum / self.power_count as f32;
            10.0 * (s / n).log10()
        } else {
            0.0
        }
    }
}


impl fmt::Debug for SyncMatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "SyncMatch {{ match_index: {}, hamming_ratio: {:7.5}, signal_deviation: {:8.5}, power_sum: {:10.7}, power_count: {:5}, saved_floor_power: {:9.7}",
            self.match_index,
            self.hamming_ratio,
            self.signal_deviation,
            self.power_sum,
            self.power_count,
            self.saved_floor_power,
        )
    }
}


impl Sync {
    pub fn new(channel: u32, preamble: u64, valid_bits: usize, samples_per_symbol: u32, sample_rate: u32) -> Self {

        // Generate preamble with samples_per_symbol repetitions for each symbol
        let mut preamble_vec = Vec::with_capacity(valid_bits as usize);

        for i in (0..valid_bits).rev() {
            let bit = ((preamble >> i) & 1) as u8;
            preamble_vec.push(bit);
        }

        let floor_tau = 2.0; // 2 seconds
        let floor_alpha = 1.0 - (-1.0 / (sample_rate as f32 * floor_tau)).exp();

        Self {
            channel: channel,
            preamble: preamble_vec,
            demod_buffer: VecDeque::new(),
            binary_buffer: VecDeque::new(),
            iq_buffer: VecDeque::new(),
            power_buffer: VecDeque::new(),
            samples_per_symbol: samples_per_symbol as usize,
            hamming_threshold: 4.0,
            matchlist: Vec::new(),
            bit_buffer: Vec::new(),
            byte_buffer: Vec::new(),
            sync_details: None,
            sample_counter: 0,
            time_offset: 0.0,
            floor_power: 0.0,
            floor_alpha: floor_alpha,
        }
    }

    pub fn update(&mut self, time_offset: f32, sample_counter: u64, sample: f32, iq_sample: Complex32) -> Option<FrameResult> {

        let adjusted_sample = match self.sync_details {
            Some(sync_match) => sample - sync_match.signal_deviation,
            None => sample,
        };

        let value = if adjusted_sample > 0.0 { 1 } else { 0 };

        self.demod_buffer.push_back(adjusted_sample);
        self.binary_buffer.push_back(value);
        self.iq_buffer.push_back(iq_sample);
        self.power_buffer.push_back(self.floor_power);

        self.sample_counter = sample_counter;
        self.time_offset = time_offset;

        match self.sync_details {
            Some(mut sync_match) => {
                sync_match.update_power(iq_sample);
                self.process_sync()
            },
            None => {
                self.update_floor_power(iq_sample);
                self.process_search();
                None
            }
        }

    }

    fn update_floor_power(&mut self, iq_sample: Complex32) {
        let power = iq_sample.norm_sqr();
        self.floor_power += self.floor_alpha * (power - self.floor_power)
    }

    fn calculate_floor_power_db(&self) -> f32 {
        10.0 * self.floor_power.max(1e-20).log10()
    }   

    #[allow(dead_code)]
    pub fn print_buffer(&self) {
        let s: String = self.binary_buffer
            .iter()
            .map(|&b| if b == 0 { '0' } else { '1' })
            .collect::<String>();
        log::info!("[{}] buffer: {}, {}, {}", self.channel, self.time_offset, self.binary_buffer.len(), s);
    }

    #[allow(dead_code)]
    pub fn print_buffer_bit(&self) {
        let s: String = self.binary_buffer
            .iter()
            .take(self.samples_per_symbol)
            .map(|&b| if b == 0 { '0' } else { '1' })
            .collect::<String>();
        log::info!("[{}] buffer: {}, {}", self.channel, self.binary_buffer.len(), s);
    }

    #[allow(dead_code)]
    pub fn print_stats(&self) {
        log::debug!("[{}] Stats: time_offset={:.6}s, sample_counter={}, demod_buffer_len={}, iq_buffer_len={}, power_buffer_len={}, binary_buffer_len={}, byte_buffer_len={}, matchlist_len={}",
            self.channel,
            self.time_offset,
            self.sample_counter,
            self.demod_buffer.len(),
            self.iq_buffer.len(),
            self.power_buffer.len(),
            self.binary_buffer.len(),
            self.byte_buffer.len(),
            self.matchlist.len(),
        );
    }

    #[allow(dead_code)]
    pub fn print_framed_buffer(&self) {

        for n in 0..self.preamble.len() {
            let s: String = self.binary_buffer
                .iter()
                .skip(n * self.samples_per_symbol)
                .take(self.samples_per_symbol)
                .map(|&b| if b == 0 { '0' } else { '1' })
                .collect();
            log::info!("[{}]    preamble[{:02}]: {}", self.channel, n, s);
        }
    }
    
    fn process_search(&mut self) {

        // we are searching every bit until we find a preamble match.

        if self.binary_buffer.len() >= self.samples_per_symbol * self.preamble.len(){

            let mut hamming_ratio = 0.0;
            let mut total_distance = 0;

            // We have enough bits in the buffer to check the entire preamble using a sliding 
            // window approach.  We short-circuit and bail early if a single symbol also fails
            // the threshold check.
            //

            for x in 0..self.preamble.len() {
                let mut distance = 0;
                for y in 0..self.samples_per_symbol {
                    let i = x * self.samples_per_symbol + y;
                    let sample_value = self.binary_buffer[i];
                    distance += (self.preamble[x] ^ sample_value) as u32;
                }
                total_distance += distance;
                hamming_ratio = total_distance as f32 / ((x + 1) as f32);
                if hamming_ratio > self.hamming_threshold {
                    break;
                }
            }

            if hamming_ratio < self.hamming_threshold {
                // we have a new match...

                let syncmatch = SyncMatch {
                    hamming_ratio: hamming_ratio,
                    match_index: self.matchlist.len(),
                    signal_deviation: self.calculcate_preamble_deviation(),
                    power_count: self.iq_buffer.len(),
                    power_sum: self.iq_buffer.iter().map(|s| s.norm_sqr()).sum(),
                    saved_floor_power: self.power_buffer[0],
                };

                log::debug!("[{}] Preamble match {:?}", self.channel, syncmatch);

                self.matchlist.push(syncmatch);

                //self.print_framed_buffer();

            } else {

                if self.matchlist.len() > 0 {
                    log::debug!("[{}] Preamble match ended after {} matches", self.channel, self.matchlist.len());

                    // because we're using a sliding window method, we may have overshot the best match.. So we need to find 
                    // the mtch and back-track and restore the bits we consumed.

                    if let Some(best_match) = self.matchlist.iter().min_by(|a, b| a.hamming_ratio.partial_cmp(&b.hamming_ratio).unwrap()) {
                        
                        let overrun_distance = self.matchlist.len() - best_match.match_index;

                        log::debug!("[{}] Best match: index={}/{}, overrun_distance={}, - {:?}",
                            self.channel,
                            best_match.match_index,
                            self.matchlist.len(),
                            overrun_distance,
                            best_match,
                        );
                        
                        self.sync_details = Some(*best_match);

                        self.binary_buffer.drain(0..self.binary_buffer.len() - overrun_distance - 1);
                        self.iq_buffer.drain(0..self.iq_buffer.len() - overrun_distance - 1);
                        self.demod_buffer.drain(0..self.demod_buffer.len() - overrun_distance - 1);
                        self.power_buffer.drain(0..self.power_buffer.len() - overrun_distance - 1);

                        self.matchlist.clear();

                    }
                }
            }

            self.binary_buffer.pop_front();
            self.iq_buffer.pop_front();
            self.demod_buffer.pop_front();
            self.power_buffer.pop_front();
        
        }
    }

    fn process_sync(&mut self) -> Option<FrameResult> {

        match self.sync_details {

            Some(sync_match) => {

                if self.binary_buffer.len() >= self.samples_per_symbol{

                    let value = self.binary_buffer.iter().take(self.samples_per_symbol).sum::<u8>();
                    let bit = if value > (self.samples_per_symbol / 2) as u8 { 1 } else { 0 };
                    self.bit_buffer.push(bit);

                    self.binary_buffer.drain(0..self.samples_per_symbol);

                    if self.bit_buffer.len() == 8 {
                        let byte = self.bit_buffer.iter().fold(0, |acc, &b| (acc << 1) | b);
                        self.byte_buffer.push(byte);
                        self.bit_buffer.clear();
                    }

                    // Validate length field early: if first byte is 0 or unreasonably large, reset sync
                    if self.byte_buffer.len() == 1 {
                        let len = self.byte_buffer[0] as usize;
                        if len < 12 || len > 127 {
                            log::warn!("[{}] Invalid packet length {}, resetting sync", self.channel, len);
                            self.sync_details = None;
                            self.byte_buffer.clear();
                            self.bit_buffer.clear();
                            return None;
                        }
                    }

                    // when we've read enough bytes to match the L Field.
                    //                
                    if self.byte_buffer.len() > 1 && self.byte_buffer.len() == (self.byte_buffer[0] as usize + 1) {

                        let crc_ok = self.check_crc(&self.byte_buffer);
                        let power_db = sync_match.calculate_final_power();
                        self.floor_power = sync_match.saved_floor_power;
                        let snr = sync_match.calculate_snr(self.floor_power);
                        let noise_db = self.calculate_floor_power_db();
                      
                        let result = Some(FrameResult{
                            channel: self.channel,
                            timestamp: chrono::Local::now().timestamp_millis() as u64, // TODO - technically incorrect.
                            packet_len: self.byte_buffer.len() as u32,
                            data: self.byte_buffer.clone(),
                            snr: snr,
                            crc_valid: crc_ok,
                            hamming_ratio: sync_match.hamming_ratio,
                            power_db: power_db,
                            noise_db: noise_db,
                        });

                        self.sync_details = None;
                        self.byte_buffer.clear();
                        self.bit_buffer.clear();

                        self.print_stats();

                        return result;
                    }
                }
                None
            },
            None => {None}
        }

    }

    fn check_crc(&self, data: &[u8]) -> bool {

        let crc = crc16::State::<crc16::EN_13757>::calculate(&data[..data.len() - 2]);
        let crc_check = (data[data.len() - 2] as u16) << 8 | data[data.len() - 1] as u16;
        log::debug!("[{}] CRC16: packet={:04X}, check={:04X}, matched={}", self.channel, crc, crc_check, crc == crc_check);
        
        crc == crc_check
    }

    // calculate the average deviation of the preamble samples from zero.  This is used to correct for 
    // for "frequency offset".

    fn calculcate_preamble_deviation(&self) -> f32 {

        let mut negative_sum = 0.0;
        let mut positive_sum = 0.0;
        let mut negative_count = 0;
        let mut positive_count = 0;

        for sample in self.demod_buffer.iter().take(self.samples_per_symbol * self.preamble.len()) {
            if *sample < 0.0 {
                negative_sum += *sample;
                negative_count += 1;
            } else {
                positive_sum += *sample;
                positive_count += 1;
            }
        }

        let negative_avg = if negative_count > 0 { negative_sum / negative_count as f32 } else { 0.0 };
        let positive_avg = if positive_count > 0 { positive_sum / positive_count as f32 } else { 0.0 };
        let deviation = (positive_avg + negative_avg) / 2.0;

        deviation

    }

}