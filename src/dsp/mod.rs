
pub mod fsk_demod;
pub mod xfirdec;
pub mod sync;
pub mod wize_demod;
pub use wize_demod::WizeDemodulator;

use std::sync::Arc;

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

#[derive(Debug, Clone)]
pub struct DemodSample {
    iq: Complex32,
    discriminator: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecodedFrame {
    pub channel: u32,
    pub bytes: Vec<u8>,
    pub crc_valid: bool,
    pub sample_error_rate: f32,
    pub noise_power: Option<f64>,
    pub signal_power: f64,
    pub snr_db: Option<f64>,
}

