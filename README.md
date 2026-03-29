# Sendspin TUI (Rust)

A terminal UI player for [Sendspin](https://github.com/Sendspin) written in Rust.

Built with [sendspin-rs](https://github.com/Sendspin/sendspin-rs) for synchronized audio playback and [ratatui](https://ratatui.rs/) for the terminal interface.

## Features

- Synchronized audio playback with drift correction
- Live track metadata (title, artist, album, progress)
- Playback controls (play/pause, next/previous, volume, mute, repeat, shuffle)
- Clock sync status monitoring

## Usage

```bash
# Connect to a Sendspin server
cargo run -- --server ws://192.168.1.100:8927/sendspin

# List available audio output devices
cargo run -- --list-audio-devices

# Use a specific audio device
cargo run -- --server ws://192.168.1.100:8927/sendspin --audio-device 0

# Set a custom player name (shown to the server)
cargo run -- --server ws://192.168.1.100:8927/sendspin --name "Living Room"
```

## Building

Requires Rust 1.75+ and a working audio backend (CoreAudio on macOS, ALSA on Linux).

```bash
cargo build --release
```

## Architecture

```
Main thread          Tokio runtime          cpal thread
(TUI render loop) <--mpsc--> (protocol + decode) --> (SyncedPlayer)
```

- **Main thread** handles terminal rendering and keyboard input
- **Tokio runtime** manages WebSocket connection, audio decoding, and command dispatch
- **cpal thread** handles low-level audio output via `SyncedPlayer`

## License

MIT
