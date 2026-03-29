mod app;
mod event;
mod protocol;
mod tui;
mod ui;

use std::fs::File;

use clap::Parser;
use sendspin_tui::shared;

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
        shared::list_devices()?;
        return Ok(());
    }

    // Create channels
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
    let (command_tx, command_rx) = tokio::sync::mpsc::unbounded_channel();

    // Spawn tokio runtime on background threads
    let rt = tokio::runtime::Runtime::new()?;
    let server = args.server.clone();
    let name = args.name.clone();
    let audio_device = args.audio_device.clone();
    rt.spawn(async move {
        protocol::run_protocol(server, name, audio_device, event_tx, command_rx).await;
    });

    // Signal handling: forward SIGINT/SIGTERM as Command::Quit
    let signal_tx = command_tx.clone();
    rt.spawn(async move {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigint = signal(SignalKind::interrupt()).expect("failed to install SIGINT handler");
        let mut sigterm =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = sigint.recv() => {}
            _ = sigterm.recv() => {}
        }
        let _ = signal_tx.send(event::Command::Quit);
    });

    // Run TUI on main thread (blocks until quit)
    let terminal = ratatui::init();
    let result = tui::run(terminal, event_rx, command_tx);
    ratatui::restore();

    result
}
