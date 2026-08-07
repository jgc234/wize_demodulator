// frequency translating DIR decimator

use num_complex::Complex32;

pub struct FreqTranslatingDirDecimator {
    decimation_factor: usize,
    phase: f64,
    phase_inc: f64,
    taps: Vec<f32>,
    delay: Vec<Complex32>,
    head: usize,
}

impl FreqTranslatingDirDecimator {
    pub fn new(offset_freq: f32, decimation_factor: usize, cutoff_freq: f32, ntaps: usize, sample_rate: f32) -> Self {
       
        Self {
            decimation_factor,
            phase: 0.0,
            phase_inc: -2.0 * std::f64::consts::PI * offset_freq as f64 / sample_rate as f64,
            taps: Self::generate_lowpass_taps(cutoff_freq, ntaps, sample_rate),
            delay: vec![Complex32::new(0.0, 0.0); ntaps],
            head: 0,
        }
    }

    fn generate_lowpass_taps(cutoff_freq: f32, ntaps: usize, sample_rate: f32) -> Vec<f32> {

        let mut taps = vec![0.0; ntaps];

        let normalised_fc = cutoff_freq / sample_rate; // Normalized cutoff frequency
        let m = (ntaps - 1) as f32 / 2.0;

        for n in 0..ntaps {
            let x = n as f32 - m;

            let sinc = if x.abs() < 1e-6 {
                2.0 * normalised_fc
            } else {
                (2.0 * std::f32::consts::PI * normalised_fc * x).sin() / (std::f32::consts::PI * x)
            };

            // Hamming Window for further sidelobe suppression
            let window = 0.54 - 0.46 * (2.0 * std::f32::consts::PI * n as f32 / (ntaps - 1) as f32).cos();

            taps[n] = sinc * window;
        }

        let gain: f32 = taps.iter().sum();
        for t in &mut taps {
            *t /= gain;
        }

        taps
    }

    fn oscillate(&mut self) -> Complex32 {

        let result = Complex32::new(self.phase.cos() as f32, self.phase.sin() as f32);

        self.phase += self.phase_inc;
        if self.phase > std::f64::consts::PI {
            self.phase -= 2.0 * std::f64::consts::PI;
        } else if self.phase < -std::f64::consts::PI {
            self.phase += 2.0 * std::f64::consts::PI;
        }

        result
    }

    pub fn process(&mut self, input: &[Complex32], output: &mut Vec<Complex32>) {

        assert!(input.len() % self.decimation_factor == 0, "Input length must be a multiple of decimation factor");

        output.clear();

        // work in chunks of decimation_factor and emit one output sample per chunk.  Each chunk will be decimated by the decimation factor.
        // A delay line has a depth matching the number of taps.
       
        for chunk in input.chunks(self.decimation_factor) {

            assert!(chunk.len() % self.decimation_factor == 0, "Input length must be a multiple of decimation factor");

            for sample in chunk {

                let translated_sample = *sample * self.oscillate();
                self.delay[self.head] = translated_sample;
                self.head = (self.head + 1) % self.delay.len();

            }

            let mut acc = Complex32::new(0.0, 0.0);

            // head is next location, which doesn't have data in it.. so we need to go back one.. 
            // which is fiddly, so pre-decrement in the loop instead.
            let mut idx = self.head;

            for tap in &self.taps {
                idx = if idx == 0 { self.delay.len() - 1 } else { idx - 1 };
                acc += self.delay[idx] * *tap;
            }

            output.push(acc);

            }
            // each chunk will emit one output sample.. but that assumes the decimation_factor is a factor of the block size.
    }                    
    
}
