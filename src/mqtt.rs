
use rumqttc::{Client, MqttOptions, QoS};
use std::time::Duration;
use std::thread::JoinHandle;
use crate::dsp::FrameResult;

pub struct MqttPublisher {
    client: Client,
    topic: String,
    _event_loop_thread: JoinHandle<()>,
}

impl MqttPublisher {

    pub fn new(mqtt_server: &str, mqtt_port: u16, mqtt_topic: String) -> Result<Self, String> {

        let mut mqttoptions = MqttOptions::new("mac", mqtt_server, mqtt_port);
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
            _event_loop_thread: event_loop_thread,
        })
    }

    pub fn publish(&self, result: &FrameResult) {

        let json = match serde_json::to_vec(&result) {
            Ok(p) => p,
            Err(e) => {
                log::error!("Unable to serialize telemetry: {}", e);
                return;
            }
        };
        log::debug!("Publishing telemetry: {}", String::from_utf8_lossy(&json));

        match self.client.publish(&self.topic, QoS::AtLeastOnce, true, json) {
            Ok(_) => {},
            Err(e) => {
                log::error!("Failed to publish telemetry: {}", e);
                return;
            }
        }

    }

}