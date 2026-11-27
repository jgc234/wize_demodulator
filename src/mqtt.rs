
use rumqttc::{Client, MqttOptions, QoS};
use crate::dsp::DecodedFrame;
use std::time::Duration;
use std::thread::JoinHandle;

pub struct MqttPublisher {
    client: Client,
    topic: String,
    host: String,
    _event_loop_thread: JoinHandle<()>,
}

impl MqttPublisher {

    pub fn new(mqtt_server: &str, mqtt_port: u16, mqtt_topic: String) -> Result<Self, String> {
        let host = gethostname::gethostname().to_string_lossy().into_owned();
        let mut mqttoptions = MqttOptions::new(host.clone(), mqtt_server, mqtt_port);
        mqttoptions.set_keep_alive(Duration::from_secs(5));
        mqttoptions.set_clean_session(false);

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
            host,
            _event_loop_thread: event_loop_thread,
        })
    }

    pub fn publish(&self, frame: &DecodedFrame) {
        let payload = frame_message(frame, &self.host, chrono::Utc::now().timestamp_millis())
            .to_string();
        log::debug!("Publishing JSON frame ({} bytes)", payload.len());

        match self.client.publish(&self.topic, QoS::AtLeastOnce, true, payload) {
            Ok(_) => {},
            Err(e) => {
                log::error!("Failed to publish JSON frame: {}", e);
                return;
            }
        }

    }

}

fn frame_message(
    frame: &DecodedFrame,
    host: &str,
    timestamp: i64,
) -> serde_json::Value {
    let data = frame
        .bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let power_db = (frame.signal_power > 0.0)
        .then(|| 10.0 * frame.signal_power.log10());
    let noise_db = frame
        .noise_power
        .filter(|power| *power > 0.0)
        .map(|power| 10.0 * power.log10());

    serde_json::json!({
        "timestamp": timestamp,
        "packet_len": frame.bytes.len(),
        "channel": frame.channel,
        "data": data,
        "host": host,
        "snr": frame.snr_db,
        "crc_valid": frame.crc_valid,
        "hamming_ratio": frame.sample_error_rate,
        "power_db": power_db,
        "noise_db": noise_db,
    })
}

#[cfg(test)]
mod tests {
    use super::frame_message;
    use crate::dsp::DecodedFrame;

    #[test]
    fn serializes_frame_and_metrics_as_json() {
        let frame = DecodedFrame {
            channel: 2,
            bytes: vec![0x21, 0x06, 0xa4],
            crc_valid: false,
            sample_error_rate: 0.25,
            noise_power: Some(0.01),
            signal_power: 100.0,
            snr_db: Some(40.0),
        };

        assert_eq!(
            frame_message(&frame, "test-host", 123),
            serde_json::json!({
                "timestamp": 123,
                "packet_len": 3,
                "channel": 2,
                "data": "2106a4",
                "host": "test-host",
                "snr": 40.0,
                "crc_valid": false,
                "hamming_ratio": 0.25,
                "power_db": 20.0,
                "noise_db": -20.0,
            })
        );
    }
}