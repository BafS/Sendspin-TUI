use std::sync::Arc;
use std::time::{Duration, Instant};

use log::{error, warn};
use sendspin::ProtocolClient;
use sendspin::audio::decode::{Decoder, PcmDecoder, PcmEndian};
use sendspin::audio::{AudioBuffer, AudioFormat, Codec, SyncedPlayer};
use sendspin::protocol::messages::{ClientCommand, ControllerCommand, GoodbyeReason, Message};
use sendspin_tui::shared;
use tokio::sync::mpsc;
use tokio::time::interval;

use crate::event::{AppEvent, Command};

enum ExitReason {
    Quit,
    Disconnected,
}

/// Connect to a Sendspin server and run the protocol loop with auto-reconnect.
pub async fn run_protocol(
    server_url: String,
    client_name: String,
    audio_device_query: Option<String>,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    mut command_rx: mpsc::UnboundedReceiver<Command>,
) {
    let mut backoff = shared::Backoff::new();

    loop {
        // Resolve device fresh each attempt
        let audio_device = match &audio_device_query {
            Some(query) => match shared::find_device(query) {
                Ok(dev) => Some(dev),
                Err(e) => {
                    let _ = event_tx.send(AppEvent::Error(format!("Audio device error: {e}")));
                    None
                }
            },
            None => None,
        };

        match run_inner(
            &server_url,
            &client_name,
            audio_device,
            &event_tx,
            &mut command_rx,
        )
        .await
        {
            Ok(ExitReason::Quit) => break,
            Ok(ExitReason::Disconnected) => {
                backoff.reset();
                let delay = backoff.next_delay();
                let _ = event_tx.send(AppEvent::Disconnected(format!(
                    "Reconnecting in {:.1}s...",
                    delay.as_secs_f64()
                )));
                tokio::time::sleep(delay).await;
            }
            Err(e) => {
                let delay = backoff.next_delay();
                let _ = event_tx.send(AppEvent::Disconnected(format!(
                    "{e} — reconnecting in {:.1}s...",
                    delay.as_secs_f64()
                )));
                tokio::time::sleep(delay).await;
            }
        }
    }
}

async fn run_inner(
    server_url: &str,
    client_name: &str,
    audio_device: Option<cpal::Device>,
    event_tx: &mpsc::UnboundedSender<AppEvent>,
    command_rx: &mut mpsc::UnboundedReceiver<Command>,
) -> Result<ExitReason, Box<dyn std::error::Error + Send + Sync>> {
    let hello = shared::build_hello(client_name, "Sendspin TUI");
    let client = ProtocolClient::connect(server_url, hello).await?;
    let (mut message_rx, mut audio_rx, clock_sync, ws_tx, _guard) = client.split();

    let device_name = {
        use cpal::traits::{DeviceTrait, HostTrait};
        let dev = audio_device
            .as_ref()
            .cloned()
            .or_else(|| cpal::default_host().default_output_device());
        dev.and_then(|d| d.description().ok().map(|desc| desc.name().to_string()))
    };
    let _ = event_tx.send(AppEvent::Connected { device_name });

    shared::send_initial_client_state(&ws_tx).await?;
    shared::send_time_sync(&ws_tx).await?;

    let mut sync_interval = interval(Duration::from_secs(5));

    let mut decoder: Option<PcmDecoder> = None;
    let mut audio_format: Option<AudioFormat> = None;
    let mut synced_player: Option<SyncedPlayer> = None;

    let mut current_volume: u8 = 100;
    let mut current_muted = false;
    let mut current_playing = false;
    let mut current_repeat = sendspin::protocol::messages::RepeatMode::Off;
    let mut current_shuffle = false;

    let exit_reason = loop {
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
                    Message::ServerCommand(cmd) => {
                        if let Some(player_cmd) = cmd.player {
                            shared::handle_player_command(&player_cmd, &synced_player);
                        }
                    }
                    Message::GroupUpdate(update) => {
                        let _ = event_tx.send(AppEvent::GroupUpdate {
                            playback_state: update.playback_state,
                            group_name: update.group_name,
                        });
                    }
                    other => {
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
                    Command::Quit => break ExitReason::Quit,
                    cmd => {
                        let result = apply_command(
                            cmd,
                            &mut current_volume,
                            &mut current_muted,
                            &mut current_playing,
                            &mut current_repeat,
                            &mut current_shuffle,
                        );
                        let _ = event_tx.send(AppEvent::LocalStateUpdate {
                            volume: current_volume,
                            muted: current_muted,
                            playing: current_playing,
                            repeat: current_repeat.clone(),
                            shuffle: current_shuffle,
                        });
                        if let Some(msg) = result
                            && let Err(e) = ws_tx.send_message(msg).await
                        {
                            error!("Failed to send command: {e}");
                            break ExitReason::Disconnected;
                        }
                    }
                }
            }
            _ = sync_interval.tick() => {
                let _ = shared::send_time_sync(&ws_tx).await;
                let sync = clock_sync.lock();
                if let Some(rtt) = sync.rtt_micros() {
                    let _ = event_tx.send(AppEvent::ClockSync {
                        rtt_ms: rtt as f64 / 1000.0,
                        quality: sync.quality(),
                    });
                }
            }
            else => {
                break ExitReason::Disconnected;
            }
        }
    };

    // Graceful shutdown
    let _ = ws_tx
        .send_message(Message::ClientGoodbye(
            sendspin::protocol::messages::ClientGoodbye {
                reason: GoodbyeReason::UserRequest,
            },
        ))
        .await;

    Ok(exit_reason)
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
