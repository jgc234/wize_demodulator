
use num_complex::Complex32;
use crate::dsp::{FrameResult, RealBlock, SampleBlock, fsk_demod, sync, xfirdec};

pub struct WizeDemodulator {
    channel: u32,
    channel_freq: u32,
    offset_freq: i32,
    samples_per_symbol: u32,
    post_sample_rate: u32,
    fir: xfirdec::FreqTranslatingDirDecimator,
    demod: fsk_demod::FskDemod,
    sync: sync::Sync,
    raw_sample_counter: u64,
    post_sample_counter: u64,
    demod_output: Vec<f32>,
    fir_output: Vec<Complex32>,
    previous_check_time: std::time::Instant,
    previous_post_sample_counter: u64,
    previous_raw_sample_counter: u64,
}

impl WizeDemodulator {
    pub fn new(
        channel: u32,
        cutoff_freq: u32,
        decimation_factor: u32,
        sdr_sample_rate: u32,
        sdr_center_freq: u32,
        symbol_rate: u32,
        ntaps: usize,
        preamble: u64,
        preamble_len: usize,
    ) -> Self {

        let post_sample_rate = sdr_sample_rate / decimation_factor;
        let samples_per_symbol = post_sample_rate / symbol_rate;
        let channel_freq = 169_393_750 + 12_500 * (channel + 1);
        let offset_freq = channel_freq as i32 - sdr_center_freq as i32;

        assert!(sdr_sample_rate % decimation_factor == 0, "Sample rate must be divisible by stage_decimator_factor");
        assert!(post_sample_rate % symbol_rate == 0, "Post-decimation sample rate must be divisible by symbol rate");
        assert!(samples_per_symbol > 0, "Samples per symbol must be greater than 0");
        assert!(ntaps > 0, "Number of FIR taps must be greater than 0");
        assert!(preamble_len > 0, "Preamble length must be greater than 0");
        assert!(preamble < (1 << preamble_len), "Preamble value must fit within preamble length");

        // this is the pipeline.. translate frequency, decimate, demodulate, sync, crc check, output frame
        let fir = xfirdec::FreqTranslatingDirDecimator::new(offset_freq as f32, decimation_factor as usize, cutoff_freq as f32, ntaps, sdr_sample_rate as f32);
        let demod = fsk_demod::FskDemod::new();
        let sync = sync::Sync::new(channel, preamble, preamble_len, samples_per_symbol, post_sample_rate);

        Self {
            channel,
            channel_freq,
            offset_freq,
            samples_per_symbol,
            post_sample_rate,
            fir,
            demod,
            sync,
            raw_sample_counter: 0,
            post_sample_counter: 0,
            demod_output: Vec::new(),
            fir_output: Vec::new(),
            previous_check_time: std::time::Instant::now(),
            previous_post_sample_counter: 0,
            previous_raw_sample_counter: 0,
        }
    }

    pub fn print_info(&self) {
        log::info!("[{}] WizeDemodulator Info:", self.channel);
        log::info!("[{}]   Channel: {}", self.channel, self.channel);
        log::info!("[{}]   Channel Frequency: {} Hz", self.channel, self.channel_freq);
        log::info!("[{}]   Offset Frequency: {} Hz", self.channel, self.offset_freq);
        log::info!("[{}]   Samples per Symbol: {}", self.channel,   self.samples_per_symbol);
        log::info!("[{}]   Post-decimation Sample Rate: {} Hz", self.channel, self.post_sample_rate);
    }

    pub fn process_block(&mut self, input: &[Complex32]) -> Option<Vec<FrameResult>> {

        let mut results = Vec::new();

        // freuqency translate, FIR and decimation in one step.
        self.fir.process(input, &mut self.fir_output);

        // demodulate..
        self.demod.process(&self.fir_output, &mut self.demod_output);

        self.raw_sample_counter += input.len() as u64;
        let mut time_offset = (self.post_sample_counter as f64 / self.post_sample_rate as f64) as f32;

        for (i, demod_sample) in self.demod_output.iter().enumerate() {

            let iq_sample = self.fir_output[i];

            time_offset = (self.post_sample_counter as f64 / self.post_sample_rate as f64) as f32;

            if let Some(frame_result) = self.sync.update(time_offset, self.post_sample_counter, *demod_sample, iq_sample) {
                log::info!("[{}] -- {:?}", self.channel, frame_result);
                results.push(frame_result);
            }

            self.post_sample_counter += 1;
        }

        let now = std::time::Instant::now();
        if (now - self.previous_check_time).as_secs_f32() >= 60.0 {

            let delta_post_samples = self.post_sample_counter - self.previous_post_sample_counter;
            let delta_raw_samples = self.raw_sample_counter - self.previous_raw_sample_counter;
            let raw_sample_rate = delta_raw_samples as f32 / (now - self.previous_check_time).as_secs_f32();
            let post_sample_rate = delta_post_samples as f32 / (now - self.previous_check_time).as_secs_f32();

            log::info!("[{}] Stats: time_offset={:.6}s, raw_sample_rate={:.2} samples/s, post_decimated_sample_rate={:.2} samples/s",
                self.channel,
                time_offset,
                raw_sample_rate,
                post_sample_rate,
            );

            self.previous_post_sample_counter = self.post_sample_counter;
            self.previous_raw_sample_counter = self.raw_sample_counter;
            self.previous_check_time = self.previous_check_time + std::time::Duration::from_secs(60);
        }

        if results.is_empty() {
            None
        } else {
            Some(results)
        }

    }

    pub fn run(&mut self, rx: std::sync::mpsc::Receiver<SampleBlock>, result_tx: std::sync::mpsc::Sender<FrameResult>) {
        while let Ok(block) = rx.recv() {
            if let Some(results) = self.process_block(&block) {
                for result in results {
                    log::debug!("[{}] sending result - {:?}", self.channel, result);
                    result_tx.send(result).unwrap();
                }
            }
        }
        log::info!("[{}] Worker exiting", self.channel);
    }
}
