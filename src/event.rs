use sendspin::audio::AudioFormat;
use sendspin::protocol::messages::{ControllerState, MetadataState, PlaybackState, RepeatMode};
use sendspin::sync::SyncQuality;

/// Events sent from the protocol task to the TUI thread.
pub enum AppEvent {
    Connected {
        device_name: Option<String>,
    },
    Disconnected(String),
    Metadata(MetadataState),
    Controller(ControllerState),
    /// Optimistic local state update after user command
    LocalStateUpdate {
        volume: u8,
        muted: bool,
        playing: bool,
        repeat: RepeatMode,
        shuffle: bool,
    },
    GroupUpdate {
        playback_state: Option<PlaybackState>,
        group_name: Option<String>,
    },
    StreamStarted(AudioFormat),
    StreamEnded,
    ClockSync {
        rtt_ms: f64,
        quality: SyncQuality,
    },
    Error(String),
}

/// Commands sent from the TUI thread to the protocol task.
#[derive(Debug)]
pub enum Command {
    PlayPause,
    Next,
    Previous,
    VolumeUp,
    VolumeDown,
    Mute,
    CycleRepeat,
    ToggleShuffle,
    Quit,
}
