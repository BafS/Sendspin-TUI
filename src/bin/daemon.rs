//! Headless Sendspin audio player daemon.
//! Connects to a server, plays audio, and reconnects automatically on disconnect.

use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::Parser;
use log::{debug, error, info, warn};
use sendspin::audio::decode::{Decoder, PcmDecoder, PcmEndian};
use sendspin::audio::{AudioBuffer, AudioFormat, Codec, SyncedPlayer};
use sendspin::protocol::messages::{GoodbyeReason, Message};
use sendspin::ProtocolClient;
use sendspin_tui::shared;
use tokio::sync::watch;
use tokio::time::interval;

/// Headless Sendspin audio player daemon
#[derive(Parser)]
#[command(name = "sendspin-daemon")]
#[command(about = "Headless Sendspin audio player with auto-reconnect")]
struct Args {
    /// WebSocket URL of the Sendspin server
    #[arg(short, long, default_value = "ws://localhost:8927/sendspin")]
    server: String,

    /// Client name shown to the server
    #[arg(short, long, default_value = "Sendspin Daemon")]
    name: String,

    /// Audio output device (index or ID string)
    #[arg(long)]
    audio_device: Option<String>,

    /// List available audio output devices and exit
    #[arg(long)]
    list_audio_devices: bool,
}

use shared::Backoff;

enum ExitReason {
    Shutdown,
    Reconnect,
}

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args = Args::parse();

    if args.list_audio_devices {
        shared::list_devices()?;
        return Ok(());
    }

    // Signal handling via watch channel
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = sigint.recv() => {}
            _ = sigterm.recv() => {}
        }
        let _ = shutdown_tx.send(true);
    });

    // Reconnection loop
    let mut backoff = Backoff::new();
    loop {
        if *shutdown_rx.borrow() {
            break;
        }

        match connect_and_run(&args, shutdown_rx.clone()).await {
            Ok(ExitReason::Shutdown) => {
                info!("Shutting down");
                break;
            }
            Ok(ExitReason::Reconnect) => {
                backoff.reset();
                let delay = backoff.next_delay();
                warn!("Disconnected, reconnecting in {:.1}s...", delay.as_secs_f64());
                tokio::time::sleep(delay).await;
            }
            Err(e) => {
                let delay = backoff.next_delay();
                error!("Connection error: {e}");
                warn!("Reconnecting in {:.1}s...", delay.as_secs_f64());
                tokio::time::sleep(delay).await;
            }
        }
    }

    Ok(())
}

async fn connect_and_run(
    args: &Args,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<ExitReason, Box<dyn std::error::Error + Send + Sync>> {
    // Resolve audio device fresh each attempt
    let audio_device = match &args.audio_device {
        Some(query) => Some(
            shared::find_device(query)
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.to_string().into() })?,
        ),
        None => None,
    };

    let hello = shared::build_hello(&args.name, "Sendspin Daemon");
    info!("Connecting to {}...", args.server);
    let client = ProtocolClient::connect(&args.server, hello).await?;
    let (mut message_rx, mut audio_rx, clock_sync, ws_tx, _guard) = client.split();

    info!("Connected to {}", args.server);

    shared::send_initial_client_state(&ws_tx).await?;
    shared::send_time_sync(&ws_tx).await?;

    let mut sync_interval = interval(Duration::from_secs(5));
    let mut decoder: Option<PcmDecoder> = None;
    let mut audio_format: Option<AudioFormat> = None;
    let mut synced_player: Option<SyncedPlayer> = None;

    loop {
        tokio::select! {
            Some(msg) = message_rx.recv() => {
                match msg {
                    Message::StreamStart(stream_start) => {
                        if let Some(ref player_config) = stream_start.player {
                            if player_config.codec != "pcm" {
                                warn!("Unsupported codec: {}", player_config.codec);
                                continue;
                            }
                            let fmt = AudioFormat {
                                codec: Codec::Pcm,
                                sample_rate: player_config.sample_rate,
                                channels: player_config.channels,
                                bit_depth: player_config.bit_depth,
                                codec_header: None,
                            };
                            info!(
                                "Stream started: PCM {}Hz/{}bit/{}ch",
                                fmt.sample_rate, fmt.bit_depth, fmt.channels
                            );
                            decoder = Some(PcmDecoder::with_endian(fmt.bit_depth, PcmEndian::Little));
                            audio_format = Some(fmt);
                        }
                    }
                    Message::StreamEnd(_) => {
                        info!("Stream ended");
                        if let Some(ref player) = synced_player {
                            player.clear();
                        }
                    }
                    Message::StreamClear(_) => {
                        if let Some(ref player) = synced_player {
                            player.clear();
                        }
                    }
                    Message::ServerState(server_state) => {
                        if let Some(ref metadata) = server_state.metadata {
                            let title = metadata.title.as_deref().unwrap_or("Unknown");
                            let artist = metadata.artist.as_deref().unwrap_or("Unknown");
                            if metadata.title.is_some() || metadata.artist.is_some() {
                                info!("Now playing: \"{title}\" by {artist}");
                            }
                        }
                        if let Some(ref controller) = server_state.controller {
                            debug!("Controller: vol={} muted={}", controller.volume, controller.muted);
                        }
                    }
                    Message::ServerCommand(cmd) => {
                        if let Some(ref player_cmd) = cmd.player {
                            shared::handle_player_command(player_cmd, &synced_player);
                            match player_cmd.command.as_str() {
                                "volume" => if let Some(v) = player_cmd.volume {
                                    info!("Volume set to {v}%");
                                },
                                "mute" => if let Some(m) = player_cmd.mute {
                                    info!("{}", if m { "Muted" } else { "Unmuted" });
                                },
                                _ => {}
                            }
                        }
                    }
                    Message::GroupUpdate(update) => {
                        if let Some(ref name) = update.group_name {
                            info!("Group: {name}");
                        }
                    }
                    _ => {}
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
                            info!("Audio output initialized");
                            synced_player = Some(player);
                        }
                        Err(e) => {
                            error!("Audio init failed: {e}");
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
            _ = sync_interval.tick() => {
                let _ = shared::send_time_sync(&ws_tx).await;
                let sync = clock_sync.lock();
                if let Some(rtt) = sync.rtt_micros() {
                    debug!("Clock sync: RTT={:.1}ms quality={:?}", rtt as f64 / 1000.0, sync.quality());
                }
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    info!("Received shutdown signal");
                    let _ = ws_tx
                        .send_message(Message::ClientGoodbye(
                            sendspin::protocol::messages::ClientGoodbye {
                                reason: GoodbyeReason::UserRequest,
                            },
                        ))
                        .await;
                    return Ok(ExitReason::Shutdown);
                }
            }
            else => {
                // All channels closed — server disconnected
                return Ok(ExitReason::Reconnect);
            }
        }
    }
}
