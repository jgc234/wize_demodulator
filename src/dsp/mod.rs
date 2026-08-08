
pub mod fsk_demod;
pub mod xfirdec;
pub mod sync;
pub mod wize_demod;
pub use wize_demod::WizeDemodulator;
use std::sync::Arc;
use std::fmt;
use std::fmt::Write;
use serde;

use num_complex::Complex32;

//pub trait ComplexBlock {
//    fn process(&mut self, input: &[Complex32], output: &mut Vec<Complex32>);
//}

pub type SampleBlock = Arc<Vec<Complex32>>;

pub trait RealBlock {
    fn process(
        &mut self,
        input: &[Complex32],
        output: &mut Vec<f32>,
    );
}

#[derive(serde::Serialize)]
pub struct FrameResult {
    pub timestamp: u64,
    pub packet_len: u32,
    pub channel: u32,
    pub data: Vec<u8>,
    pub snr: f32,
    pub crc_valid: bool,
    pub hamming_ratio: f32,
    pub power_db: f32,
    pub noise_db: f32,
}


impl fmt::Debug for FrameResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {

        let mut hex_string = String::with_capacity(self.data.len() * 2);
        for b in &self.data {
            write!(&mut hex_string, "{:02x}", b).unwrap();
        }

        write!(
            f,
            "FrameResult {{ channel: {}, timestamp: {}, crc_valid: {:5}, hamming_ratio: {:5.3}, snr: {:6.3}, power_db: {:6.3}, noise_db: {:6.3}, data: {}",
            self.channel,
            self.timestamp,
            self.crc_valid,
            self.hamming_ratio,
            self.snr,
            self.power_db,
            self.noise_db,
            hex_string,
        )
    }
}










