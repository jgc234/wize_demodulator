use num_complex::Complex32;

use crate::dsp::RealBlock;
use crate::dsp::DemodSample;

pub struct FskDemod {
    previous: Option<Complex32>,
}

impl FskDemod {
    pub fn new() -> Self {
        Self {
            previous: None,
        }
    }
}

impl RealBlock for FskDemod {
    
    fn process(&mut self, input: &[Complex32], output: &mut Vec<DemodSample>) {
        output.clear();

        for sample in input {
            if let Some(prev) = self.previous {
                let delta = *sample * prev.conj();
                let value = delta.im.atan2(delta.re);

                let demod_sample = DemodSample{
                    iq: *sample,
                    discriminator: value,
                };
                output.push(demod_sample);
            }

            self.previous = Some(*sample);
        }
    }
}

