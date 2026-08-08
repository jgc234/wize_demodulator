mod args;
mod iq;
mod iq_writer;
mod rtl_tcp;
mod dsp;

use anyhow::{Result, anyhow};
use clap::Parser;
use log;
use num_complex::Complex32;
use std::fs::File;
use std::io::Read;
use args::Args;
use dsp::{WizeDemodulator,};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use dsp::FrameResult;
use dsp::SampleBlock;
use std::io::Write;
use chrono::Local;
use std::fs::OpenOptions;


enum InputSource {
    Rtl(rtl_tcp::RtlTcp),
    File(File),
}

enum OutputSource {
    File(File),
    None,
}

fn read_samples(source: &mut InputSource, buffer: &mut [u8]) -> Result<usize> {
    match source {
        InputSource::Rtl(rtl) => {
            rtl.read_samples(buffer)?;
            Ok(buffer.len())
        }
        InputSource::File(file) => Ok(file.read(buffer)?),
    }
}

fn main() -> Result<()> {

    env_logger::Builder::from_default_env()
        .format(|buf, record| {
            writeln!(
                buf,
                "{} [{:<5}] - {}",
                Local::now().format("%Y-%m-%d %H:%M:%S"),
                record.level(),
                record.args()
            )
        })
        .init();
    
    let args = Args::parse();

    let (mut input, sdr_sample_rate, sdr_center_freq) = if let Some(path) = args.input_file.as_ref() {
        log::info!("Reading CU8 input from {}", path.display());
        (
            InputSource::File(File::open(path)?),
            args.sample_rate,
            args.frequency,
        )
    } else {
        let addr = args
            .addr
            .ok_or_else(|| anyhow!("either --addr or --input-file must be provided"))?;

        log::info!("Connecting to RTL-TCP at {}", addr);

        let rtl = rtl_tcp::RtlTcp::connect(addr, &args)?;
        let rtl_sample_rate = rtl.sample_rate;
        let rtl_center_freq = rtl.center_freq;

        log::info!("RTL-TCP connection established");

        (
            InputSource::Rtl(rtl),
            rtl_sample_rate,
            rtl_center_freq,
        )
    };

    
    let mut output: OutputSource = match args.output_file.as_ref() {
        Some(path) => {
            log::info!("Writing JSON output to {}", path.display());
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            OutputSource::File(file)
        }
        None => OutputSource::None,
    };

    let sdr_sample_rate = sdr_sample_rate;
    let cutoff_freq = args.cutoff_freq;
    let decimation_factor = args.decimation_factor;
    let symbol_rate = 2_400;
    let ntaps = args.ntaps;
    let preamble = 0x5555F672;
    let preamble_len = 32;
    let block_size = args.block_size;

    log::info!("wize_demodulator Info:");
    log::info!("  SDR Sample Rate: {} Hz", sdr_sample_rate);
    log::info!("  SDR Center Frequency: {} Hz", sdr_center_freq);
    log::info!("  Block size: {} samples", block_size);
    log::info!("  Cutoff Frequency: {} Hz", cutoff_freq);
    log::info!("  Decimation Factor: {}", decimation_factor);
    log::info!("  Symbol Rate: {} Hz", symbol_rate);
    log::info!("  Number of FIR Taps: {}", ntaps);
    log::info!("  Preamble: {:#X}", preamble);
    log::info!("  Preamble Length: {} bits", preamble_len);

    struct Worker {
        tx: mpsc::SyncSender<SampleBlock>,
        handle: thread::JoinHandle<()>,
    }

    let mut workers = Vec::new();

    let (result_tx, result_rx) = mpsc::channel::<FrameResult>();

    for wize_channel in [0,2,3,4,5] {

        let (tx, rx) = mpsc::sync_channel::<SampleBlock>(4);
        let local_result_tx = result_tx.clone();
        
        let handle = thread::spawn(move || {

            let mut wize_demod = WizeDemodulator::new(
                wize_channel,
                cutoff_freq,
                decimation_factor,
                sdr_sample_rate,
                sdr_center_freq,
                symbol_rate,
                ntaps,
                preamble,
                preamble_len,
            );

            wize_demod.print_info();
            wize_demod.run(rx, local_result_tx);
            log::info!("Worker for channel {} exiting", wize_channel);
        });

        workers.push(Worker { tx, handle });
        
    }
    
    let _ = thread::spawn(move || {
        while let Ok(result) = result_rx.recv() {
            match &mut output {
                OutputSource::None => {},
                OutputSource::File(json_output) => {
                    let json = serde_json::to_string(&result).unwrap();
                    writeln!(json_output, "{}", json).unwrap();
                }
            }
        }
    });

    let mut raw = vec![0u8; block_size];
    let mut delta_time;
    let mut previous_check_time = std::time::Instant::now();
    let mut previous_raw_sample_counter: u64 = 0;
    let mut raw_sample_counter: u64 = 0;
    let mut block_counter: u64 = 0;

    loop {
        block_counter += 1;

        let bytes_read = read_samples(&mut input, &mut raw)?;
        if bytes_read == 0 {
            log::info!("Input ended");
            break;
        }

        let mut samples = Vec::<Complex32>::with_capacity(raw.len() / 2);

        iq::iq_to_complex(&raw, &mut samples);
        raw_sample_counter += samples.len() as u64;

        let block = Arc::new(samples);

        for worker in &workers {
            worker.tx.send(block.clone()).unwrap();
        }

        // drop our reference
        drop(block);

        let now = std::time::Instant::now();
        delta_time = (now - previous_check_time).as_secs_f32();
        if delta_time >= 10.0 {
            let delta_samples = raw_sample_counter - previous_raw_sample_counter;
            let sample_rate = delta_samples as f32 / delta_time;
            log::info!("{} samples processed in {:.3} seconds, sample rate: {:.3} samples/sec, block_counter={}",
                delta_samples, delta_time, sample_rate, block_counter);
            previous_raw_sample_counter = raw_sample_counter;
            previous_check_time = previous_check_time + std::time::Duration::from_secs(10);
        }
    }

    for worker in workers.drain(..) {
        drop(worker.tx);          // Receiver will see EOF
        worker.handle.join().unwrap();
    }

    Ok(())

}