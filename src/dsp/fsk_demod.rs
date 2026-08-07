use num_complex::Complex32;

use crate::dsp::RealBlock;

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
    
    fn process(
        &mut self,
        input: &[Complex32],
        output: &mut Vec<f32>,
    ) {
        output.clear();

        for sample in input {
            if let Some(prev) = self.previous {
                let delta = *sample * prev.conj();

                output.push(delta.im.atan2(delta.re));

            }

            self.previous = Some(*sample);
        }
    }
}

