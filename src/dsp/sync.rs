
use std::{collections::VecDeque};
use log;
use std::fmt;

use crate::dsp::DemodSample;

//use crc16;
//use chrono;
//use crate::dsp::{FrameResult};
//use num_complex::Complex32;
//use std::fmt;
//use gethostname::gethostname;

enum SyncEvent {
    NoMatch,
    Match,
}

// Sync and Frame need dc_offset.
// searching could includ a window offset..
// as soon as we sync, truncate buffer.
// we can truncate while in search.. but as soon as we're in sync state, we
// can't truncate until we get a selected candidate.
//

#[derive(PartialEq)]
enum SyncState {
    Searching,
    Preamble,
    Sync,
    Frame,
}

#[derive(Copy,Clone,PartialEq)]
struct PreambleCandidate {
    buffer_window_start: usize,
    buffer_window_end: usize,
    dc_offset: f32,
    symbol_0_ramp: f32,
    symbol_1_ramp: f32,
    symbol_ramp: f32,
    hamming_distance: u32,
    hamming_rate: f32,
}

impl fmt::Debug for PreambleCandidate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PreambleCandidate {{ buffer_window: {}-{}, dc_offset: {:+.4}, symbol_0_ramp: {:+.4}, symbol_1_ramp: {:+.4}, symbol_ramp: {:+.4}, hamming_distance: {:+.4}, hamming_rate: {:.4} }}",
            self.buffer_window_start, self.buffer_window_end, self.dc_offset,
            self.symbol_0_ramp, self.symbol_1_ramp, self.symbol_ramp,
            self.hamming_distance, self.hamming_rate,
        )
    }
}

pub struct Sync {
    state: SyncState,
    channel: u32,
    preamble_bit_vec: Vec<u8>,
    sync_byte_vec: Vec<u8>,
    buffer: VecDeque<DemodSample>,
    search_offset: usize,
    samples_per_symbol: f32,
    preamble_candidates: Vec<PreambleCandidate>,
    collected_frame: Vec<u8>,
    byte_offset: usize,
}


impl Sync {

    pub fn new(channel: u32, samples_per_symbol: f32) -> Self {

        // assumed preable is 0x5555.
        // Generate preamble with samples_per_symbol repetitions for each symbol
        // build a vector of sine wave that repreents the preamble.
        let preamble_size: usize = 16;
        let preamble_value: u32 = 0x5555;
        let sync_value: u32= 0xF672;
        let sync_size: usize = 2;

        let preamble_bit_vec: Vec<u8> = (0..preamble_size)
            .rev()
            .map(|i| ((preamble_value >> i) & 1) as u8)
            .collect();

        let sync_byte_vec: Vec<u8> = (0..sync_size)
            .rev()
            .map(|i| ((sync_value >> (8*i)) & 0xFF) as u8)
            .collect();

        log::info!("Preamble for channel {}: size={} vec={:?}", channel, preamble_bit_vec.len(), preamble_bit_vec);

        Sync{
            state : SyncState::Searching,
            channel: channel,
            preamble_bit_vec: preamble_bit_vec,
            sync_byte_vec: sync_byte_vec,
            buffer: VecDeque::new(),
            search_offset: 0,
            samples_per_symbol: samples_per_symbol,
            preamble_candidates: Vec::new(),
            collected_frame: Vec::new(),
            byte_offset: 0,
        }
    }

    pub fn update(&mut self, demod_block: &[DemodSample]) {
        self.buffer.extend(demod_block.iter().cloned());
        self.process()
    }

    fn process_searching(&mut self) -> SyncEvent {

        // this is a more detailed check on a specific offset for a sliding window in the buffer.

        let mut symbol_0_hamming_distance = 0;
        let mut symbol_1_hamming_distance = 0;
        let mut symbol_0_leading_sum = 0.0;
        let mut symbol_1_leading_sum = 0.0;
        let mut symbol_0_trailing_sum = 0.0;
        let mut symbol_1_trailing_sum = 0.0;
        let mut preamble_hamming_distance = 0;

        // TODO - fix hardcoded preamble length.
        for n in 0..self.preamble_bit_vec.len() {

            let symbol_start = self.search_offset + (n as f32 * self.samples_per_symbol) as usize;
            let symbol_end = self.search_offset + ((n as f32 + 1.0) * self.samples_per_symbol) as usize;
            let expected_bit = n as u8 % 2;
            let mut symbol_hamming_distance = 0;
            let leading_value = self.buffer[symbol_start].discriminator;
            let trailing_value = self.buffer[symbol_end - 1].discriminator;
                            
            let mut output = String::new();

            // iterate samples over a single symbol.
            for j in symbol_start..symbol_end {
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
                return SyncEvent::NoMatch;
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
        let leading_dc_offset = (symbol_0_leading_avg + symbol_1_leading_avg) / 2.0;
        let trailing_dc_offset = (symbol_0_trailing_avg + symbol_1_trailing_avg) / 2.0;
        let dc_offset = (symbol_0_dc_offset + symbol_1_dc_offset) / 2.0;
        let ramp_diff = symbol_0_ramp - symbol_1_ramp;
        let ramp_diff_abs = ramp_diff.abs();
        let hamming_rate = preamble_hamming_distance as f32 / self.preamble_bit_vec.len() as f32;

        log::info!("    [{}] Preamble offset {:04} - symbol_0 - ε{:2} ◀{:+.4} ▶{:+.4} μ{:+.4}, ramp {:+.4}, symbol_1 - ε{:2} ◀{:+.4} ▶{:+.4} μ{:+.4} ramp {:+.4}, overall  - ε{:2} ◀{:+.4} ▶{:+.4} μ{:+.4} ramp Δ{:+.4} abs Δ{:+.4}",
            self.channel, self.buffer_offset,
            symbol_0_hamming_distance, symbol_0_leading_avg, symbol_0_trailing_avg, symbol_0_dc_offset, symbol_0_ramp,
            symbol_1_hamming_distance, symbol_1_leading_avg, symbol_1_trailing_avg, symbol_1_dc_offset, symbol_1_ramp,
            preamble_hamming_distance, leading_dc_offset, trailing_dc_offset, dc_offset, ramp_diff, ramp_diff_abs
        );

        let candidate = PreambleCandidate{
            buffer_window_start: self.search_offset,
            buffer_window_end: (self.search_offset as f32 + self.samples_per_symbol * self.preamble_bit_vec.len() as f32) as usize,
            dc_offset: dc_offset,
            symbol_0_ramp: symbol_0_ramp,
            symbol_1_ramp: symbol_1_ramp,
            symbol_ramp: ramp_diff_abs,
            hamming_distance: preamble_hamming_distance,
            hamming_rate: hamming_rate,
        };

        self.preamble_candidates.push(candidate);
        self.search_offset += 1;

        SyncEvent::Match

    }


    // two options for processing:
    // - match on each state.. then process as many samples as possible under that state.
    // - or .. process each sample.. and swithch on the state.

    fn extract_byte(&mut self) -> u8 {

        let mut byte_value = 0;

        for i in 0..8_usize {
            let start = (self.byte_offset as f32 * 8.0 * self.samples_per_symbol) as usize;
            let end = ((self.byte_offset + 1) as f32 * 8.0 * self.samples_per_symbol) as usize;
            let total: f32 = self.buffer.range(start..end)
                .map(|x| x.discriminator)
                .sum();
            let avg = total / (end - start) as f32;
            let bit_value = (avg >= 0.0) as u8;
            byte_value = (byte_value<<1) | (bit_value & 0x01);
        }
        self.byte_offset += 1;

        byte_value

    }

    fn process_sync(&mut self) -> SyncEvent {

        // process a byte at a time and collect.
        SyncEvent::NoMatch
    }

    fn process_frame(&mut self) -> SyncEvent {
        SyncEvent::NoMatch
    }

    pub fn process(&mut self) {

        // greedy states - each state can consume a number of samples until it 
        // yeilds or changes state.

        // strategy - loop for at least one symbol worth of bits.. based on
        // current state, consume bits.. but for the preamble state, when do we
        // drain the buffer? only when working on preamble. how do we know when
        // to give up on preamble? should each state drain their own bits?

        // some strategy options - make the state machine a tight loop, and
        // do the state changes in other functions.  this is somewhat unclean.
        // a traditional "event causes state change method mmight be better,
        // but that creates a large and messy loop that also has to include the
        // logic for buffer 

        // lets go with the large state machine model, with functions returning
        // "events".. since each state has 3 different outcomes (keep going,
        // move forward, or return to searching).. this means we can't easily
        // use Option. Two potential options here;
        // 1 - create Event types that return lots of different options and
        // handle the state transitions in the main loop.. or
        // 2 - push the state transitions into the other functions..

        // must be at least 2 bytes worth of symbols available in the buffer.

        // hwo do we manage buffer offests for various states.. ideally for
        // non-integer samples per symbol we want to calculate the offset for
        // each symbol edge based on a calculation from the start of the
        // buffer..  ideally this point should be the known edge after the
        // preamble.

        // rules.. onece in sync frame stats, the buffer_offset is the end of
        // the preamble, or the start of the first sync bit.
        
        while self.buffer.len() > (self.buffer_offset as f32 + self.samples_per_symbol * 2.0) as usize {

            let event = match &self.state {
                SyncState::Searching => self.process_searching(),
                SyncState::Preamble => self.process_searching(),
                SyncState::Sync => self.process_sync(),
                SyncState::Frame => self.process_frame(),
            };

            let new_state = match (&self.state, event) {
                (SyncState::Searching, SyncEvent::NoMatch) => {
                    self.buffer.pop_front();
                    SyncState::Searching
                },
                (SyncState::Searching, SyncEvent::Match) => {
                    SyncState::Preamble
                },
                (SyncState::Preamble, SyncEvent::NoMatch) => {
                    let candidate_option = self.select_candidate();
                    match candidate_option {
                        Some(candidate) => {
                            self.buffer.drain(0..self.buffer_offset);
                            self.buffer_offset = 0;
                            SyncState::Sync
                        },
                        None => {
                            SyncState::Searching
                        }
                    }
                },
                (SyncState::Preamble, SyncEvent::Match) => {
                    SyncState::Preamble
                },
                (SyncState::Sync, SyncEvent::Match) => {
                    SyncState::Frame
                },
                (SyncState::Sync, SyncEvent::NoMatch) => {
                    SyncState::Searching
                },
                (SyncState::Frame, SyncEvent::Match) => {
                    SyncState::Frame
                },
                (SyncState::Frame, SyncEvent::NoMatch) => {
                    SyncState::Searching
                },
            };

            self.state = new_state;
            
        }
    }

        // remove all samples up to the last 20 samples to keep some overlap for next block

    fn select_candidate(&mut self) -> Option<PreambleCandidate> {
        return  self.preamble_candidates.iter().min_by(|a, b| a.symbol_ramp.partial_cmp(&b.symbol_ramp).unwrap()).copied();
    }
}

