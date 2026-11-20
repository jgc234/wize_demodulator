
use num_complex::Complex32;
use crate::dsp::{FrameResult, RealBlock, SampleBlock, fsk_demod, sync, xfirdec};

pub struct WizeDemodulator {
    channel: u32,
    channel_freq: u32,
    offset_freq: i32,
    output_sample_rate: u32,
    samples_per_symbol: f32,
    fir: xfirdec::FreqTranslatingDirDecimator,
    demod: fsk_demod::FskDemod,
    sync: sync::Sync,
    demod_output: Vec<f32>,
    fir_output: Vec<Complex32>,
}

impl WizeDemodulator {
    pub fn new(
        channel: u32,
        cutoff_freq: u32,
        decimation_factor: u32,
        input_sample_rate: u32,
        input_center_freq: u32,
        symbol_rate: u32,
        ntaps: usize,
        preamble: u64,
        preamble_len: usize,
    ) -> Self {

        let output_sample_rate = input_sample_rate / decimation_factor;
        let samples_per_symbol = output_sample_rate as f32 / symbol_rate as f32;
        let channel_freq = 169_393_750 + 12_500 * (channel + 1);
        let offset_freq = channel_freq as i32 - input_center_freq as i32;

        assert!(input_sample_rate % decimation_factor == 0, "Sample rate must be divisible by stage_decimator_factor");
        //assert!(output_sample_rate % symbol_rate == 0, "Post-decimation sample rate must be divisible by symbol rate");
        assert!(samples_per_symbol > 0.0, "Samples per symbol must be greater than 0");
        assert!(ntaps > 0, "Number of FIR taps must be greater than 0");
        assert!(preamble_len > 0, "Preamble length must be greater than 0");
        assert!(preamble < (1 << preamble_len), "Preamble value must fit within preamble length");

        // this is the pipeline.. translate frequency, decimate, demodulate, sync, crc check, output frame
        let fir = xfirdec::FreqTranslatingDirDecimator::new(offset_freq as f32, decimation_factor as usize, cutoff_freq as f32, ntaps, input_sample_rate as f32);
        let demod = fsk_demod::FskDemod::new();
        let sync = sync::Sync::new(channel, samples_per_symbol);
 
        Self {
            channel,
            channel_freq,
            offset_freq,
            output_sample_rate,
            samples_per_symbol,
            fir,
            demod,
            sync,
            demod_output: Vec::new(),
            fir_output: Vec::new(),
        }
    }

    pub fn print_info(&self) {
        log::info!("[{}] WizeDemodulator Info:", self.channel);
        log::info!("[{}]   Channel: {}", self.channel, self.channel);
        log::info!("[{}]   Channel Frequency: {} Hz", self.channel, self.channel_freq);
        log::info!("[{}]   Offset Frequency: {} Hz", self.channel, self.offset_freq);
        log::info!("[{}]   Samples per Symbol: {}", self.channel,   self.samples_per_symbol);
        log::info!("[{}]   Post-decimation Sample Rate: {} Hz", self.channel, self.output_sample_rate);
    }

    pub fn process_block(&mut self, input: &[Complex32]) -> Option<Vec<FrameResult>> {

        // freuqency translate, FIR and decimation in one step.
        self.fir.process(input, &mut self.fir_output);

        // demodulate..
        self.demod.process(&self.fir_output, &mut self.demod_output);

        self.sync.update(&self.demod_output);

        None

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
