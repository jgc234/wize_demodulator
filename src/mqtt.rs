
use rumqttc::{Client, MqttOptions, QoS};
use gethostname::gethostname;
use serde;
use std::time::Duration;
use std::thread::JoinHandle;
use crate::dsp::FrameResult;

pub struct MqttPublisher {
    client: Client,
    topic: String,
    _event_loop_thread: JoinHandle<()>,
}

#[derive(Debug, serde::Serialize)]
struct PacketResult {
    timestamp: u64,
    packet_len: u32,
    channel: u32,
    data: String,
    host: String,
    snr: f32,
    crc_valid: bool,
    hamming_ratio: f32,
    power_db: f32,
    noise_db: f32,
}

impl MqttPublisher {

    pub fn new(mqtt_server: &str, mqtt_port: u16, mqtt_topic: String) -> Result<Self, String> {

        let mut mqttoptions = MqttOptions::new("rumqtt-sync", mqtt_server, mqtt_port);
        mqttoptions.set_keep_alive(Duration::from_secs(5));

        let (client, mut connection) = Client::new(mqttoptions, 10);

        let event_loop_thread = std::thread::spawn(move || {
            loop {
                match connection.iter().next() {
                    Some(Ok(notification)) => {
                        log::debug!("MQTT Notification: {:?}", notification);
                    }
                    Some(Err(e)) => {
                        log::warn!("MQTT connection error: {}", e);
                        // Sleep to prevent tight loop on connection errors
                        std::thread::sleep(Duration::from_millis(500));
                    }
                    None => {
                        log::info!("MQTT connection closed");
                        break;
                    }
                }
            }
        });

        Ok(Self{
            client: client,
            topic: mqtt_topic,
            _event_loop_thread: event_loop_thread,
        })
    }

    pub fn publish(&self, result: &FrameResult) {

        let mut hexstring = String::new();
        for byte in &result.frame {
            hexstring.push_str(&format!("{:02x}", byte));
        }

        let packet = PacketResult {
            timestamp: result.timestamp_ms,
            packet_len: result.frame.len() as u32,
            channel: result.channel,
            data: hexstring,
            host: gethostname().to_string_lossy().into_owned(),
            snr: result.snr,
            crc_valid: result.crc_valid,
            hamming_ratio: result.hamming_ratio,
            power_db: result.power_db,
            noise_db: result.noise_db,
        };

        let json = match serde_json::to_vec(&packet) {
            Ok(p) => p,
            Err(e) => {
                log::error!("Unable to serialize telemetry: {}", e);
                return;
            }
        };
        log::debug!("Publishing telemetry: {}", String::from_utf8_lossy(&json));

        match self.client.publish(&self.topic, QoS::AtLeastOnce, false, json) {
            Ok(_) => {},
            Err(e) => {
                log::error!("Failed to publish telemetry: {}", e);
                return;
            }
        }

    }

}