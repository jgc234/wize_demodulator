
use num_complex::Complex32;
use crate::dsp::{DecodedFrame, DemodSample, RealBlock, SampleBlock, fsk_demod, sync, xfirdec};

pub struct WizeDemodulator {
    channel: u32,
    channel_freq: u32,
    offset_freq: i32,
    output_sample_rate: u32,
    samples_per_symbol: f32,
    fir: xfirdec::FreqTranslatingDirDecimator,
    demod: fsk_demod::FskDemod,
    sync: sync::Sync,
    demod_output: Vec<DemodSample>,
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
        preamble: u16,
        sync_word: u16,
    ) -> Self {

        assert!(decimation_factor > 0, "Decimation factor must be greater than zero");
        assert!(symbol_rate > 0, "Symbol rate must be greater than zero");
        assert!(input_sample_rate % decimation_factor == 0, "Sample rate must be divisible by stage_decimator_factor");
        let output_sample_rate = input_sample_rate / decimation_factor;
        let samples_per_symbol = output_sample_rate as f32 / symbol_rate as f32;
        let channel_freq = 169_393_750 + 12_500 * (channel + 1);
        let offset_freq = channel_freq as i32 - input_center_freq as i32;

        assert!(samples_per_symbol > 0.0, "Samples per symbol must be greater than 0");
        assert!(output_sample_rate >= symbol_rate, "Sample rate must provide at least one sample per symbol");
        assert!(ntaps > 0, "Number of FIR taps must be greater than 0");

        // this is the pipeline.. translate frequency, decimate, demodulate,
        // sync, crc check, output frame 
        let fir = xfirdec::FreqTranslatingDirDecimator::new(offset_freq as f32, decimation_factor as usize, cutoff_freq as f32, ntaps, input_sample_rate as f32);
        let demod = fsk_demod::FskDemod::new();
        let sync = sync::Sync::new(
            channel,
            output_sample_rate,
            symbol_rate,
            preamble,
            sync_word,
        );
 
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

    pub fn process_block(&mut self, input: &[Complex32]) -> Vec<DecodedFrame> {

        // freuqency translate, FIR and decimation in one step.
        self.fir.process(input, &mut self.fir_output);

        // demodulate..
        self.demod.process(&self.fir_output, &mut self.demod_output);

        self.sync.update(&self.demod_output)

    }

    pub fn run(&mut self, rx: std::sync::mpsc::Receiver<SampleBlock>, result_tx: std::sync::mpsc::Sender<DecodedFrame>) {
        while let Ok(block) = rx.recv() {
            for frame in self.process_block(&block) {
                let frame_hex = frame
                    .bytes
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                log::info!("[{}] sending raw frame - {}", self.channel, frame_hex);
                result_tx.send(frame).unwrap();
            }
        }
        log::info!("[{}] Worker exiting", self.channel);
    }
}
