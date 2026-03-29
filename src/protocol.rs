use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use log::{error, warn};
use sendspin::audio::decode::{Decoder, PcmDecoder, PcmEndian};
use sendspin::audio::{AudioBuffer, AudioFormat, Codec, SyncedPlayer};
use sendspin::protocol::messages::{
    AudioFormatSpec, ClientCommand, ClientHello, ClientState, ClientTime, ControllerCommand,
    DeviceInfo, GoodbyeReason, Message, PlayerCommand, PlayerState, PlayerSyncState,
    PlayerV1Support,
};
use sendspin::ProtocolClient;
use tokio::sync::mpsc;
use tokio::time::interval;

use crate::event::{AppEvent, Command};

/// Connect to a Sendspin server and run the protocol loop.
/// Handles audio decoding, clock sync, and user commands until quit or disconnect.
pub async fn run_protocol(
    server_url: String,
    client_name: String,
    audio_device: Option<cpal::Device>,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    mut command_rx: mpsc::UnboundedReceiver<Command>,
) {
    if let Err(e) = run_inner(server_url, client_name, audio_device, &event_tx, &mut command_rx).await {
        let _ = event_tx.send(AppEvent::Disconnected(e.to_string()));
    }
}

async fn run_inner(
    server_url: String,
    client_name: String,
    audio_device: Option<cpal::Device>,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    command_rx: &mut mpsc::UnboundedReceiver<Command>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let client_id = uuid::Uuid::new_v4().to_string();

    let hello = ClientHello {
        client_id,
        name: client_name,
        version: 1,
        supported_roles: vec![
            "player@v1".to_string(),
            "controller@v1".to_string(),
            "metadata@v1".to_string(),
        ],
        device_info: Some(DeviceInfo {
            product_name: Some("Sendspin TUI".to_string()),
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
    };

    let client = ProtocolClient::connect(&server_url, hello).await?;
    let (mut message_rx, mut audio_rx, clock_sync, ws_tx, _guard) = client.split();

    let _ = event_tx.send(AppEvent::Connected);

    // Send initial client state
    let client_state = Message::ClientState(ClientState {
        player: Some(PlayerState {
            state: PlayerSyncState::Synchronized,
            volume: Some(100),
            muted: Some(false),
        }),
    });
    ws_tx.send_message(client_state).await?;

    // Send initial clock sync
    send_time_sync(&ws_tx).await?;

    // Periodic clock sync interval
    let mut sync_interval = interval(Duration::from_secs(5));

    // Audio state
    let mut decoder: Option<PcmDecoder> = None;
    let mut audio_format: Option<AudioFormat> = None;
    let mut synced_player: Option<SyncedPlayer> = None;

    // Controller state tracking (for stateful commands like volume +/-)
    let mut current_volume: u8 = 100;
    let mut current_muted = false;
    let mut current_playing = false;
    let mut current_repeat = sendspin::protocol::messages::RepeatMode::Off;
    let mut current_shuffle = false;

    loop {
        tokio::select! {
            Some(msg) = message_rx.recv() => {
                match msg {
                    Message::StreamStart(stream_start) => {
                        if let Some(ref player_config) = stream_start.player {
                            if player_config.codec != "pcm" {
                                let _ = event_tx.send(AppEvent::Error(
                                    format!("Unsupported codec: {}", player_config.codec),
                                ));
                                continue;
                            }

                            let fmt = AudioFormat {
                                codec: Codec::Pcm,
                                sample_rate: player_config.sample_rate,
                                channels: player_config.channels,
                                bit_depth: player_config.bit_depth,
                                codec_header: None,
                            };

                            decoder = Some(PcmDecoder::with_endian(fmt.bit_depth, PcmEndian::Little));
                            let _ = event_tx.send(AppEvent::StreamStarted(fmt.clone()));
                            audio_format = Some(fmt);
                        }
                    }
                    Message::StreamEnd(_) => {
                        if let Some(ref player) = synced_player {
                            player.clear();
                        }
                        let _ = event_tx.send(AppEvent::StreamEnded);
                    }
                    Message::StreamClear(_) => {
                        if let Some(ref player) = synced_player {
                            player.clear();
                        }
                    }
                    Message::ServerState(server_state) => {
                        if let Some(ref metadata) = server_state.metadata {
                            if let Some(ref progress) = metadata.progress {
                                current_playing = progress.playback_speed > 0;
                            }
                            if let Some(repeat) = metadata.repeat.clone() {
                                current_repeat = repeat;
                            }
                            if let Some(shuffle) = metadata.shuffle {
                                current_shuffle = shuffle;
                            }
                        }
                        if let Some(ref controller) = server_state.controller {
                            current_volume = controller.volume;
                            current_muted = controller.muted;
                        }
                        if let Some(metadata) = server_state.metadata {
                            let _ = event_tx.send(AppEvent::Metadata(metadata));
                        }
                        if let Some(controller) = server_state.controller {
                            let _ = event_tx.send(AppEvent::Controller(controller));
                        }
                    }
                    // ServerTime is handled internally by the library — never forwarded
                    Message::ServerCommand(cmd) => {
                        if let Some(player_cmd) = cmd.player {
                            handle_player_command(&player_cmd, &synced_player);
                        }
                    }
                    Message::GroupUpdate(update) => {
                        let _ = event_tx.send(AppEvent::GroupUpdate {
                            playback_state: update.playback_state,
                            group_name: update.group_name,
                        });
                    }
                    other => {
                        // Truncate debug output to avoid flooding
                        let debug = format!("{other:?}");
                        let truncated = if debug.len() > 120 {
                            format!("{}...", &debug[..120])
                        } else {
                            debug
                        };
                        let _ = event_tx.send(AppEvent::Error(format!("Unhandled: {truncated}")));
                    }
                }
            }
            Some(chunk) = audio_rx.recv() => {
                let Some(ref fmt) = audio_format else { continue };
                let Some(ref dec) = decoder else { continue };

                // Frame sanity check
                let bytes_per_sample = fmt.bit_depth as usize / 8;
                let frame_size = bytes_per_sample * fmt.channels as usize;
                if chunk.data.len() % frame_size != 0 {
                    continue;
                }

                let samples = match dec.decode(&chunk.data) {
                    Ok(s) => s,
                    Err(e) => {
                        warn!("Decode error: {e}");
                        continue;
                    }
                };

                // Lazy init SyncedPlayer
                if synced_player.is_none() {
                    match SyncedPlayer::new(
                        fmt.clone(),
                        Arc::clone(&clock_sync),
                        audio_device.as_ref().cloned(),
                        100,
                        false,
                    ) {
                        Ok(player) => {
                            log::info!("Audio output initialized");
                            synced_player = Some(player);
                        }
                        Err(e) => {
                            let _ = event_tx.send(AppEvent::Error(format!("Audio init failed: {e}")));
                            continue;
                        }
                    }
                }

                if let Some(ref player) = synced_player {
                    player.enqueue(AudioBuffer {
                        timestamp: chunk.timestamp,
                        play_at: Instant::now(),
                        samples,
                        format: fmt.clone(),
                    });
                }
            }
            Some(cmd) = command_rx.recv() => {
                match cmd {
                    Command::Quit => break,
                    cmd => {
                        let result = apply_command(
                            cmd,
                            &mut current_volume,
                            &mut current_muted,
                            &mut current_playing,
                            &mut current_repeat,
                            &mut current_shuffle,
                        );
                        // Update TUI with optimistic state
                        let _ = event_tx.send(AppEvent::LocalStateUpdate {
                            volume: current_volume,
                            muted: current_muted,
                            playing: current_playing,
                            repeat: current_repeat.clone(),
                            shuffle: current_shuffle,
                        });
                        if let Some(msg) = result {
                            if let Err(e) = ws_tx.send_message(msg).await {
                                error!("Failed to send command: {e}");
                                break;
                            }
                        }
                    }
                }
            }
            _ = sync_interval.tick() => {
                let _ = send_time_sync(&ws_tx).await;
                // Report clock sync status (ServerTime is handled internally by the library)
                let sync = clock_sync.lock();
                if let Some(rtt) = sync.rtt_micros() {
                    let _ = event_tx.send(AppEvent::ClockSync {
                        rtt_ms: rtt as f64 / 1000.0,
                        quality: sync.quality(),
                    });
                }
            }
            else => break,
        }
    }

    // Graceful shutdown
    let _ = ws_tx
        .send_message(Message::ClientGoodbye(
            sendspin::protocol::messages::ClientGoodbye {
                reason: GoodbyeReason::UserRequest,
            },
        ))
        .await;

    Ok(())
}

fn handle_player_command(cmd: &PlayerCommand, player: &Option<SyncedPlayer>) {
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

/// Apply a user command: mutate local state optimistically and return the protocol message.
fn apply_command(
    cmd: Command,
    volume: &mut u8,
    muted: &mut bool,
    playing: &mut bool,
    repeat: &mut sendspin::protocol::messages::RepeatMode,
    shuffle: &mut bool,
) -> Option<Message> {
    use sendspin::protocol::messages::RepeatMode;

    let (command_str, vol, mute_val) = match cmd {
        Command::PlayPause => {
            if *playing {
                *playing = false;
                ("pause", None, None)
            } else {
                *playing = true;
                ("play", None, None)
            }
        }
        Command::Next => ("next", None, None),
        Command::Previous => ("previous", None, None),
        Command::VolumeUp => {
            *volume = volume.saturating_add(5).min(100);
            ("volume", Some(*volume), None)
        }
        Command::VolumeDown => {
            *volume = volume.saturating_sub(5);
            ("volume", Some(*volume), None)
        }
        Command::Mute => {
            *muted = !*muted;
            ("mute", None, Some(*muted))
        }
        Command::CycleRepeat => {
            let cmd_str = match repeat {
                RepeatMode::Off => {
                    *repeat = RepeatMode::All;
                    "repeat_all"
                }
                RepeatMode::All => {
                    *repeat = RepeatMode::One;
                    "repeat_one"
                }
                RepeatMode::One => {
                    *repeat = RepeatMode::Off;
                    "repeat_off"
                }
            };
            (cmd_str, None, None)
        }
        Command::ToggleShuffle => {
            *shuffle = !*shuffle;
            if *shuffle {
                ("shuffle", None, None)
            } else {
                ("unshuffle", None, None)
            }
        }
        Command::Quit => return None,
    };

    Some(Message::ClientCommand(ClientCommand {
        controller: Some(ControllerCommand {
            command: command_str.to_string(),
            volume: vol,
            mute: mute_val,
        }),
    }))
}

async fn send_time_sync(
    ws_tx: &sendspin::protocol::client::WsSender,
) -> Result<(), sendspin::error::Error> {
    // SystemTime is always after UNIX_EPOCH on any reasonable system
    let client_transmitted = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_micros() as i64;
    ws_tx
        .send_message(Message::ClientTime(ClientTime { client_transmitted }))
        .await
}
