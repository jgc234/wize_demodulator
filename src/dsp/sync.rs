
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
    FoundPreamble,
    CollectingFrame,
}

pub struct Sync {
    state: SyncState,
    channel: u32,
    preamble: Vec<u8>,
    demod_buffer: VecDeque<f32>,
    samples_per_symbol: f32,
}


impl Sync {
    pub fn new(channel: u32, preamble: u64, valid_bits: usize, samples_per_symbol: f32) -> Self {

        // Generate preamble with samples_per_symbol repetitions for each symbol
        let mut preamble_vec = Vec::with_capacity(valid_bits as usize);

        for i in (0..valid_bits).rev() {
            let bit = ((preamble >> i) & 1) as u8;
            preamble_vec.push(bit);
        }

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

    pub fn process(&mut self) {

        // look for preamble in demod_buffer
        let preamble_len = self.preamble.len();
        let buffer_len = self.demod_buffer.len();

        if buffer_len < preamble_len {
            return;
        }
        
    }

}