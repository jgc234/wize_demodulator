
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
        output: &mut Vec<DemodSample>,
    );
}

#[derive(Clone, serde::Serialize)]
pub struct FrameResult {
    pub channel: u32,
    pub host: String,
    pub timestamp_ms: u64,
    pub frame: Vec<u8>,
    pub snr: f32,
    pub time_offset: f32,
    pub crc_valid: bool,
    pub hamming_ratio: f32,
    pub power_db: f32,
    pub noise_db: f32,
    pub gps_lat: Option<f64>,
    pub gps_lon: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct DemodSample {
    iq: Complex32,
    discriminator: f32,
}


impl fmt::Debug for FrameResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {

        let mut hex_string = String::with_capacity(self.frame.len() * 2);
        for b in &self.frame {
            write!(&mut hex_string, "{:02x}", b).unwrap();
        }

        write!(
            f,
            "FrameResult {{ channel: {}, timestamp_ms: {}, time_offset: {:10.5}, crc_valid: {:5}, hamming_ratio: {:5.3}, snr: {:6.3}, power_db: {:6.3}, noise_db: {:6.3}, lat: {}, lon: {}, frame: {} }}",
            self.channel,
            self.timestamp_ms,
            self.time_offset,
            self.crc_valid,
            self.hamming_ratio,
            self.snr,
            self.power_db,
            self.noise_db,
            self.gps_lat.map_or("None".to_string(), |lat| format!("{:.6}", lat)),
            self.gps_lon.map_or("None".to_string(), |lon| format!("{:.6}", lon)),
            hex_string,
        )
    }
}










