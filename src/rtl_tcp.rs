use anyhow::Result;
use log::info;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};

use crate::args::Args;

// may be unused
 
const CMD_SET_FREQUENCY: u8             = 0x01;
const CMD_SET_SAMPLE_RATE: u8           = 0x02;
const CMD_SET_GAIN_MODE: u8             = 0x03;
const _CMD_SET_GAIN: u8                  = 0x04;
const CMD_SET_FREQUENCY_CORRECTION: u8  = 0x05;
const _CMD_SET_IF_STAGE: u8              = 0x06;
const _CMD_SET_TEST_MODE: u8             = 0x07;
const CMD_SET_AGC_MODE: u8              = 0x08;
const CMD_SET_DIRECT_SAMPLING: u8       = 0x09;
const CMD_SET_OFFSET_TUNING: u8         = 0x0A;
const _CMD_SET_RTL_CRYSTAL: u8           = 0x0B;
const _CMD_SET_TUNER_CRYSTAL: u8         = 0x0C;
const CMD_SET_TUNER_GAIN_BY_INDEX: u8   = 0x0D;
const _CMD_SET_FREQ_HI32: u8             = 0x56;

pub struct RtlTcp {
    stream: TcpStream,
    pub sample_rate: u32,
    pub center_freq: u32,
}

impl RtlTcp {
    pub fn connect(addr: SocketAddr, args: &Args) -> Result<Self> {

        info!("Connecting to {}", addr);

        let mut stream = TcpStream::connect(addr)?;

        stream.set_nodelay(true)?;

        info!("Connected to RTL-TCP socket");

        // RTL-TCP header
        let mut header = [0u8; 12];
        stream.read_exact(&mut header)?;

        let magic = std::str::from_utf8(&header[0..4]).unwrap_or("????");
        let tuner = u32::from_be_bytes(header[4..8].try_into().unwrap());
        let gain_count = u32::from_be_bytes(header[8..12].try_into().unwrap());

        info!(
            "RTL header: magic={} tuner={} gain_count={}",
            magic, tuner, gain_count
        );

        send_cmd(&mut stream, CMD_SET_OFFSET_TUNING, 0)?;
        send_cmd(&mut stream, CMD_SET_GAIN_MODE, 1)?;
        send_cmd(&mut stream, CMD_SET_TUNER_GAIN_BY_INDEX, args.gain as u32)?;
        send_cmd(&mut stream, CMD_SET_FREQUENCY, args.frequency)?;
        send_cmd(&mut stream, CMD_SET_SAMPLE_RATE, args.sample_rate)?;
        send_cmd(&mut stream, CMD_SET_FREQUENCY_CORRECTION, args.ppm as u32)?;
        send_cmd(&mut stream, CMD_SET_AGC_MODE, 0)?;
        send_cmd(&mut stream, CMD_SET_DIRECT_SAMPLING, 0)?;

        info!("RTL-TCP configuration sent");

        Ok(Self { stream, sample_rate: args.sample_rate, center_freq: args.frequency })
    }

    pub fn read_samples(&mut self, buffer: &mut [u8]) -> Result<()> {
        self.stream.read_exact(buffer)?;
        Ok(())
    }

}

fn send_cmd(stream: &mut TcpStream, cmd: u8, value: u32) -> Result<()> {
    let mut buf = [0u8; 5];

    buf[0] = cmd;
    buf[1..5].copy_from_slice(&value.to_be_bytes());

    stream.write_all(&buf)?;

    Ok(())
}