mod args;
mod iq;
mod iq_writer;
mod rtl_tcp;
mod dsp;
mod mqtt;

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
use dsp::xfirdec::FreqTranslatingDirDecimator;

enum InputSource {
    Rtl(rtl_tcp::RtlTcp),
    File(File),
}

enum OutputSource {
    Mqtt(mqtt::MqttPublisher),
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

    let mut output = if let Some(mqtt_server) = args.mqtt_server.as_ref() {
        log::info!("Publishing to MQTT server {}:{}", mqtt_server, args.mqtt_port);
        let publisher = mqtt::MqttPublisher::new(
            mqtt_server,
            args.mqtt_port,
            args.mqtt_topic,
        ).map_err(|e| anyhow!("Failed to initialize MQTT publisher: {}", e))?;
        OutputSource::Mqtt(publisher)
    } else {
        OutputSource::None
    };


    let stage1_offset = 37_500;
    let stage1_cutoff_freq = 50_000;
    let stage1_decimation_factor = 8;
    let stage1_ntaps = 513;
    let stage1_sample_rate = sdr_sample_rate;

    let stage2_centre_freq = sdr_center_freq + stage1_offset;
    let stage2_sample_rate = stage1_sample_rate / stage1_decimation_factor;
    let stage2_cutoff_freq = 5_000;
    let stage2_decimation_factor = 4;
    let stage2_symbol_rate = 2_400;
    let stage2_ntaps = 257;

    let stage3_sample_rate = stage2_sample_rate / stage2_decimation_factor;

    let preamble = 0x5555F672;
    let preamble_len = 32;
    let block_size = 16_384;
    
    log::info!("wize_demodulator Info:");
    log::info!("  stage1 SDR Sample Rate: {} Hz", stage1_sample_rate);
    log::info!("  stage1 Cutoff Frequency: {} Hz", stage1_cutoff_freq);
    log::info!("  stage1 Decimation Factor: {}", stage1_decimation_factor);
    log::info!("  stage1 Number of FIR Taps: {}", stage1_ntaps);
    log::info!("  stage1 Center Frequency: {} Hz", stage1_offset);
    log::info!("  --");
    log::info!("  stage2 SDR Sample Rate: {} Hz", stage2_sample_rate);
    log::info!("  stage2 Center Frequency: {} Hz", stage2_centre_freq);
    log::info!("  stage2 Symbol Rate: {} Hz", stage2_symbol_rate);
    log::info!("  stage2 Number of FIR Taps: {}", stage2_ntaps);
    log::info!("  stage2 Cutoff Frequency: {} Hz", stage2_cutoff_freq);
    log::info!("  stage2 Decimation Factor: {}", stage2_decimation_factor);
    log::info!("  --");
    log::info!("  stage3 sample rate: {} Hz", stage3_sample_rate);
    log::info!("  Symbol Rate: {} Hz", stage2_symbol_rate);
    log::info!("  Preamble: {:#X}", preamble);
    log::info!("  Preamble Length: {} bits", preamble_len);
    log::info!("  Block size: {} samples", block_size);

    assert!(stage1_sample_rate % stage1_decimation_factor == 0, "Sample rate must be divisible by stage_decimator_factor");
    assert!(stage1_sample_rate % stage2_symbol_rate == 0, "Post-decimation sample rate must be divisible by symbol rate");
    assert!(stage1_sample_rate / stage1_decimation_factor > 0, "Samples per symbol must be greater than 0");
    assert!(stage1_ntaps > 0, "Number of FIR taps must be greater than 0");
    assert!(preamble_len > 0, "Preamble length must be greater than 0");
    assert!(preamble < (1 << preamble_len), "Preamble value must fit within preamble length");
    
    let mut stage1_fir = FreqTranslatingDirDecimator::new(
        stage1_offset as f32, //center_freq,
        stage1_decimation_factor as usize, //decimation,
        stage1_cutoff_freq as f32, // cutoff_freq,
        stage1_ntaps, //ntaps,
        sdr_sample_rate as f32,
    );


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
                stage2_cutoff_freq,
                stage2_decimation_factor,
                stage2_sample_rate,
                stage2_centre_freq,
                stage2_symbol_rate,
                stage2_ntaps,
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
                OutputSource::Mqtt(mqtt) => mqtt.publish(&result),
                OutputSource::None => {},
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
        let mut stage2_samples = Vec::<Complex32>::with_capacity(raw.len() / 2 / stage1_decimation_factor as usize);

        iq::iq_to_complex(&raw, &mut samples);
        raw_sample_counter += samples.len() as u64;

        stage1_fir.process(&samples, &mut stage2_samples);

        let block = Arc::new(stage2_samples);

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
            log::debug!("{} samples processed in {:.3} seconds, sample rate: {:.3} samples/sec, block_counter={}",
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