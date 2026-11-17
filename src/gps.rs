use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct GpsPosition {
    pub latitude: f64,
    pub longitude: f64,
    pub timestamp: DateTime<Utc>,
}

use std::sync::{Arc, RwLock};
use std::thread;
use std::time::Duration;
use nmea::Nmea;
use serialport;
use std::io::{BufRead, BufReader};

pub type SharedGps = Arc<RwLock<Option<GpsPosition>>>;

pub fn start_gps_thread(device: &str, baud: u32) -> SharedGps {

    let position: SharedGps = Arc::new(RwLock::new(None));
    let shared = Arc::clone(&position);
    let device = device.to_string();

    thread::spawn(move || {
        let port = match serialport::new(device, baud)
            .timeout(Duration::from_secs(1))
            .open()
        {
            Ok(port) => port,
            Err(e) => {
                eprintln!("GPS serial error: {e}");
                return;
            }
        };

        let mut reader = BufReader::new(port);
        let mut parser = Nmea::default();
        let mut line = String::new();

        loop {
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if let Ok(fix) = parser.parse_for_fix(line.trim()) {
                        if fix.is_valid() {
                            if let (Some(lat), Some(lon)) = (parser.latitude, parser.longitude) {
                                let new_position = GpsPosition {
                                    latitude: lat,
                                    longitude: lon,
                                    timestamp: chrono::Utc::now(),
                                };

                                *shared.write().unwrap() = Some(new_position);
                            }
                        }
                    }
                }
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    continue;
                }
                Err(e) => {
                    eprintln!("GPS read error: {e}");
                    break;
                }
            }
        }
    });

    position
}