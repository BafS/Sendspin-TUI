# Sendspin TUI (Rust)

A terminal UI player and headless daemon for [Sendspin](https://github.com/Sendspin) written in Rust.

Built with [sendspin-rs](https://github.com/Sendspin/sendspin-rs) for synchronized audio playback and [ratatui](https://ratatui.rs/) for the terminal interface.

## Binaries

This crate provides two binaries:

- **sendspin-tui** — Interactive terminal UI with track info, controls, and progress bar
- **sendspin-daemon** — Headless player with auto-reconnect, suitable for running as a service

## Features

- Synchronized audio playback with drift correction
- Live track metadata (title, artist, album, progress)
- Playback controls (play/pause, next/previous, volume, mute, repeat, shuffle)
- Clock sync status monitoring
- Graceful shutdown on SIGINT/SIGTERM
- Auto-reconnect with exponential backoff
- Stale connection detection

## Usage

### TUI

```bash
# Connect to a Sendspin server
cargo run --bin sendspin-tui -- --server ws://192.168.1.100:8927/sendspin

# Set a custom player name
cargo run --bin sendspin-tui -- --server ws://... --name "Living Room"
```

### Daemon

```bash
# Run the headless player
cargo run --bin sendspin-daemon -- --server ws://192.168.1.100:8927/sendspin

# With custom name and audio device
cargo run --bin sendspin-daemon -- --server ws://... --name "Kitchen" --audio-device 0

# Debug logging
RUST_LOG=debug cargo run --bin sendspin-daemon -- --server ws://...
```

### Common options

```bash
# List available audio output devices
cargo run --bin sendspin-tui -- --list-audio-devices

# Use a specific audio device (by index or ID)
cargo run --bin sendspin-tui -- --server ws://... --audio-device 0
```
## Building

Requires Rust 1.75+ and a working audio backend (CoreAudio on macOS, ALSA on Linux).

```bash
cargo build --release
```

Binaries are output to `target/release/sendspin-tui` and `target/release/sendspin-daemon`.

## Architecture

```
                  TUI                              Daemon
┌──────────────────────────────┐   ┌──────────────────────────────┐
│ Main thread    Tokio runtime │   │        Tokio runtime         │
│ (render loop) <--> (protocol)│   │ (protocol + reconnect loop)  │
└──────────────────────────────┘   └──────────────────────────────┘
                    │                              │
                    └──── shared (lib.rs) ─────────┘
                              │
                     cpal (SyncedPlayer)
```

Both binaries share connection setup, audio decoding, and player command handling via a common library module.

## License

MIT
