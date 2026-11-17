use clap::{ArgGroup, Parser};
use std::net::SocketAddr;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(group(
    ArgGroup::new("input")
        .required(true)
        .args(["addr", "input_file"])
))]
pub struct Args {
    /// RTL-TCP socket address (IPv4 or IPv6)
    #[arg(long, conflicts_with = "input_file")]
    pub addr: Option<SocketAddr>,

    /// Input CU8 file path (interleaved I/Q uint8 pairs)
    #[arg(long, conflicts_with = "addr")]
    pub input_file: Option<PathBuf>,

    #[arg(long, default_value_t = 169_400_000)]
    pub frequency: u32,

    #[arg(long, default_value_t = 1_536_000)]
    pub sample_rate: u32,

    #[arg(long, default_value_t = 23)]
    pub gain: i32,

    #[arg(long, default_value_t = 0)]
    pub ppm: i32,

    #[arg(long)]
    pub mqtt_server: Option<String>,

    #[arg(long, default_value_t = 1883)]
    pub mqtt_port: u16,

    #[arg(long, default_value_t = String::from("wize"))]
    pub mqtt_topic: String,

    #[arg(short, long)]
    pub passive: bool,

    // Optional GPS serial device path for location.
    #[arg(long)]
    pub gps: Option<String>,

    // Optional GPS serial baud rate (default: 9600)
    #[arg(long, default_value_t = 9600)]
    pub gps_baud: u32,

}
