mod app;
mod event;
mod protocol;
mod tui;
mod ui;

use std::fs::File;

use clap::Parser;
use cpal::traits::{DeviceTrait, HostTrait};

/// Sendspin TUI audio player
#[derive(Parser)]
#[command(name = "sendspin-tui")]
#[command(about = "Terminal UI player for Sendspin")]
struct Args {
    /// WebSocket URL of the Sendspin server
    #[arg(short, long, default_value = "ws://localhost:8927/sendspin")]
    server: String,

    /// Client name shown to the server
    #[arg(short, long, default_value = "Sendspin TUI")]
    name: String,

    /// Audio output device (index or ID string)
    #[arg(long)]
    audio_device: Option<String>,

    /// List available audio output devices and exit
    #[arg(long)]
    list_audio_devices: bool,
}

fn main() -> color_eyre::Result<()> {
    // Install panic/error hooks before anything else
    color_eyre::install()?;

    // Log to file since TUI owns the terminal
    let log_file = File::create("sendspin-tui.log")?;
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn"))
        .target(env_logger::Target::Pipe(Box::new(log_file)))
        .init();

    let args = Args::parse();

    if args.list_audio_devices {
        list_devices()?;
        return Ok(());
    }

    // Resolve audio device before entering TUI
    let device = match &args.audio_device {
        Some(query) => Some(find_device(query)?),
        None => None,
    };

    // Create channels
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
    let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();

    // Spawn tokio runtime on background threads
    let rt = tokio::runtime::Runtime::new()?;
    let server = args.server.clone();
    let name = args.name.clone();
    rt.spawn(async move {
        protocol::run_protocol(server, name, device, event_tx, command_rx).await;
    });

    // Run TUI on main thread (blocks until quit)
    let terminal = ratatui::init();
    let result = tui::run(terminal, event_rx, command_tx);
    ratatui::restore();

    result
}

fn list_devices() -> color_eyre::Result<()> {
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

fn find_device(query: &str) -> color_eyre::Result<cpal::Device> {
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
