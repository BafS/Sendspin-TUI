//! Shared utilities used by both the TUI and daemon binaries.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cpal::traits::{DeviceTrait, HostTrait};
use sendspin::audio::SyncedPlayer;
use sendspin::protocol::messages::{
    AudioFormatSpec, ClientHello, ClientState, ClientTime, DeviceInfo, Message, PlayerCommand,
    PlayerState, PlayerSyncState, PlayerV1Support,
};

/// Build a `ClientHello` message with player, controller, and metadata roles.
pub fn build_hello(client_name: &str, product_name: &str) -> ClientHello {
    ClientHello {
        client_id: uuid::Uuid::new_v4().to_string(),
        name: client_name.to_string(),
        version: 1,
        supported_roles: vec![
            "player@v1".to_string(),
            "controller@v1".to_string(),
            "metadata@v1".to_string(),
        ],
        device_info: Some(DeviceInfo {
            product_name: Some(product_name.to_string()),
            manufacturer: Some("Sendspin".to_string()),
            software_version: Some(env!("CARGO_PKG_VERSION").to_string()),
        }),
        player_v1_support: Some(PlayerV1Support {
            supported_formats: vec![
                AudioFormatSpec {
                    codec: "pcm".to_string(),
                    channels: 2,
                    sample_rate: 48000,
                    bit_depth: 24,
                },
                AudioFormatSpec {
                    codec: "pcm".to_string(),
                    channels: 2,
                    sample_rate: 48000,
                    bit_depth: 16,
                },
            ],
            buffer_capacity: 50 * 1024 * 1024,
            supported_commands: vec!["volume".to_string(), "mute".to_string()],
        }),
        artwork_v1_support: None,
        visualizer_v1_support: None,
    }
}

/// Send the initial `ClientState` message (synchronized, volume 100, unmuted).
pub async fn send_initial_client_state(
    ws_tx: &sendspin::protocol::client::WsSender,
) -> Result<(), sendspin::error::Error> {
    ws_tx
        .send_message(Message::ClientState(ClientState {
            player: Some(PlayerState {
                state: PlayerSyncState::Synchronized,
                volume: Some(100),
                muted: Some(false),
            }),
        }))
        .await
}

/// Send a clock sync (`ClientTime`) message.
pub async fn send_time_sync(
    ws_tx: &sendspin::protocol::client::WsSender,
) -> Result<(), sendspin::error::Error> {
    let client_transmitted = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_micros() as i64;
    ws_tx
        .send_message(Message::ClientTime(ClientTime { client_transmitted }))
        .await
}

/// Apply a server player command (volume/mute) to the local `SyncedPlayer`.
pub fn handle_player_command(cmd: &PlayerCommand, player: &Option<SyncedPlayer>) {
    let Some(player) = player else { return };
    match cmd.command.as_str() {
        "volume" => {
            if let Some(vol) = cmd.volume {
                player.set_volume(vol);
            }
        }
        "mute" => {
            if let Some(muted) = cmd.mute {
                player.set_mute(muted);
            }
        }
        _ => {}
    }
}

/// Exponential backoff for reconnection (100ms to 30s, 2x factor).
pub struct Backoff {
    current: Duration,
    min: Duration,
    max: Duration,
}

impl Default for Backoff {
    fn default() -> Self {
        Self::new()
    }
}

impl Backoff {
    pub fn new() -> Self {
        Self {
            current: Duration::from_millis(100),
            min: Duration::from_millis(100),
            max: Duration::from_secs(30),
        }
    }

    pub fn next_delay(&mut self) -> Duration {
        let delay = self.current;
        self.current = (self.current * 2).min(self.max);
        delay
    }

    pub fn reset(&mut self) {
        self.current = self.min;
    }
}

/// List available audio output devices.
pub fn list_devices() -> color_eyre::Result<()> {
    let mut index = 0u32;
    for host_id in cpal::available_hosts() {
        let host = cpal::host_from_id(host_id)?;
        for device in host.devices()? {
            let has_output = device
                .supported_output_configs()
                .map(|mut c| c.next().is_some())
                .unwrap_or(false);
            if !has_output {
                continue;
            }
            let id = device
                .id()
                .map_or_else(|_| "unknown".into(), |id| id.to_string());
            let desc = device
                .description()
                .map(|d| format!("{d:?}"))
                .unwrap_or_else(|_| "no description".into());
            println!("[{index:>2}] {id}\n     {desc}");
            index += 1;
        }
    }
    if index == 0 {
        println!("No output devices found.");
    }
    Ok(())
}

/// Find an audio output device by index or ID string.
pub fn find_device(query: &str) -> color_eyre::Result<cpal::Device> {
    let idx_query = query.parse::<usize>().ok();
    let mut usable_index = 0usize;

    for host_id in cpal::available_hosts() {
        let host = cpal::host_from_id(host_id)?;
        for device in host.devices()? {
            let has_output = device
                .supported_output_configs()
                .map(|mut c| c.next().is_some())
                .unwrap_or(false);
            if !has_output {
                continue;
            }
            if let Some(idx) = idx_query {
                if usable_index == idx {
                    return Ok(device);
                }
            } else {
                let id = device
                    .id()
                    .map_or_else(|_| String::new(), |id| id.to_string());
                if id == query {
                    return Ok(device);
                }
            }
            usable_index += 1;
        }
    }
    Err(color_eyre::eyre::eyre!("Audio device '{query}' not found"))
}
