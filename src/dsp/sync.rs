
use std::{collections::VecDeque};
use log;
//use crc16;
//use chrono;
//use crate::dsp::{FrameResult};
//use num_complex::Complex32;
//use std::fmt;
//use gethostname::gethostname;

enum SyncState {
    Searching,
    PreambleWindow,
    FoundPreamble,
    CollectingFrame,
}

#[derive(Debug)]
struct PreambleCandidate {
    buffer_offset: usize,
    dc_offset: f32,
    symbol_0_ramp: f32,
    symbol_1_ramp: f32,
    hamming_distance: u32,
    symbol_avg_ramp: f32,
}

pub struct Sync {
    state: SyncState,
    channel: u32,
    preamble: Vec<u8>,
    demod_buffer: VecDeque<f32>,
    samples_per_symbol: f32,
}


impl Sync {

    pub fn new(channel: u32, samples_per_symbol: f32) -> Self {

        // assumed preable is 0x5555.. match with a sine wave for x cycles.
        // Generate preamble with samples_per_symbol repetitions for each symbol
        // build a vector of sine wave that repreents the preamble.
        let preamble_size: usize = 16;
        let mut preamble_vec = Vec::with_capacity(samples_per_symbol as usize * preamble_size);

        for i in 0..(preamble_size as f32 * samples_per_symbol) as usize {
            let bit = ((i / samples_per_symbol as usize) % 2) as u8;
            preamble_vec.push(bit);
        }

        log::info!("Preamble for channel {}: {:?}", channel, preamble_vec);
        log::info!("premable size {}: {}", channel, preamble_vec.len());
    
        Self {
            channel: channel,
            preamble: preamble_vec,
            demod_buffer: VecDeque::new(),
            state: SyncState::Searching,
            samples_per_symbol: samples_per_symbol,
        }
    }

    pub fn update(&mut self, demod_block: &Vec<f32>) {

        self.demod_buffer.extend(demod_block);
        self.process()

    }

    fn check_sync_candidate(&self, offset: usize) -> Option<PreambleCandidate> {

        // this is a more detailed check on a specific offset for a sliding window in the buffer.

        let mut symbol_0_distance = 0;
        let mut symbol_1_distance = 0;
        let mut symbol_0_leading_sum = 0.0;
        let mut symbol_1_leading_sum = 0.0;
        let mut symbol_0_trailing_sum = 0.0;
        let mut symbol_1_trailing_sum = 0.0;
        let mut preamble_distance = 0;

        // TODO - fix hardcoded preamble length.
        for n in 0..16 {

            let start = offset + (n as f32 * self.samples_per_symbol) as usize;
            let end = offset + ((n as f32 + 1.0) * self.samples_per_symbol) as usize;
            let expected_bit = n as u8 % 2;
            let mut symbol_distance = 0;
            let leading_value = self.demod_buffer[start];
            let trailing_value = self.demod_buffer[end - 1];
                            
            let mut output = String::new();

            for j in start..end {
                let value = self.demod_buffer[j];
                let binary_value = (value >= 0.0) as u8;
                let d = (binary_value ^ expected_bit) as u32;
                symbol_distance += d;
                let ch = if binary_value == 0 { '.'} else { '█' };
                output.push(ch);
            }

            preamble_distance += symbol_distance;

            if expected_bit == 0 {
                symbol_0_distance += symbol_distance;
                symbol_0_leading_sum += leading_value;
                symbol_0_trailing_sum += trailing_value;
            } else {
                symbol_1_distance += symbol_distance;
                symbol_1_leading_sum += leading_value;
                symbol_1_trailing_sum += trailing_value;
            }

            log::info!("     [{}] Symbol {:2}, symbol[{}] - ε{:2} ◀{:+.4} ▶{:+.4} samples: [{}]", 
                self.channel, n,
                expected_bit,
                symbol_distance,
                leading_value, trailing_value,
                output
            );
            // if the symbol distance is a fail, bailout and return.
            // 

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
        let ramp_diff = (symbol_0_ramp - symbol_1_ramp);

        log::info!("    [{}] Preamble - symbol_0 - ε{:2} ◀{:+.4} ▶{:+.4} μ{:+.4}, ramp {:+.4}",
            self.channel,
            symbol_0_distance, symbol_0_leading_avg, symbol_0_trailing_avg, symbol_0_dc_offset, symbol_0_ramp,
        );
        log::info!("    [{}] Preamble - symbol_1 - ε{:2} ◀{:+.4} ▶{:+.4} μ{:+.4} ramp {:+.4}",
            self.channel,
            symbol_1_distance, symbol_1_leading_avg, symbol_1_trailing_avg, symbol_1_dc_offset, symbol_1_ramp,
        );
        log::info!("    [{}] Preamble - overall  - ε{:2} ◀{:+.4} ▶{:+.4} μ{:+.4} ramp Δ{:+.4}",
            self.channel,
            preamble_distance, leading_offset, trailing_offset, offset, ramp_diff,
        );

        Some(PreambleCandidate{
            buffer_offset: offset,
            dc_offset: dc_offset,
            symbol_0_ramp: symbol_0_ramp,
            symbol_1_ramp: symbol_1_ramp,
            hamming_distance: preamble_distance,
            symbol_avg_ramp: ramp_diff,
        })

    }


    pub fn process(&mut self) {

        // look for preamble in demod_buffer

        if self.demod_buffer.len() < self.preamble.len() + self.samples_per_symbol as usize {
            // not enough samples to process
            return;
        }

        let mut running_count = 0;

        for i in 0..(self.demod_buffer.len() - self.preamble.len()) {

            let mut distance = 0;
            for j in 0..self.preamble.len() {
                let binary_value = (self.demod_buffer[i+j] >= 0.0) as u8;
                distance += (binary_value ^ self.preamble[j]) as u32;
            }

            let error_rate = distance as f32 / self.preamble.len() as f32;

            if error_rate <= 0.125 {
                if running_count == 0 {
                    log::info!("[{}] Starting to collect frame", self.channel);
                }
                log::info!("   [{}] Preamble found at index {} with distance {}, error rate {:.4}", self.channel, i, distance, error_rate);
                running_count += 1;
            } else {
                if running_count > 0 {
                    log::info!("[{}] Stopping frame collection at index {} with run {} and distance {}", self.channel, i, running_count, distance);
                    running_count = 0;
                }
                continue;
            }
            // do deeper assessment of preamble match.. look at each symbol and look of dc-offset, zero crossings, and per-symbol hamming distance..

            let result = self.check_sync_candidate(i);

            log::info!(" - result {:?}", result);

        }

        // remove all samples up to the last 20 samples to keep some overlap for next block
        let remove_count = self.demod_buffer.len() - self.preamble.len();
        self.demod_buffer.drain(0..remove_count);
        
    }

}