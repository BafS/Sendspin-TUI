use std::time::Instant;

use sendspin::audio::AudioFormat;
#[cfg(test)]
use sendspin::protocol::messages::{ControllerState, MetadataState, TrackProgress};
use sendspin::protocol::messages::{PlaybackState, RepeatMode};
use sendspin::sync::SyncQuality;

use crate::event::AppEvent;

/// TUI application state, owned exclusively by the main thread.
pub struct AppState {
    // Track metadata
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,

    // Progress
    pub track_progress_ms: i64,
    pub track_duration_ms: i64,
    pub playback_speed: i32,
    pub progress_updated_at: Instant,

    // Controller
    pub volume: u8,
    pub muted: bool,
    pub repeat: RepeatMode,
    pub shuffle: bool,

    // Group
    pub playback_state: PlaybackState,
    pub group_name: Option<String>,

    // Connection/sync
    pub connected: bool,
    pub device_name: Option<String>,
    pub last_data_at: Option<Instant>,
    pub sync_rtt_ms: Option<f64>,
    pub sync_quality: Option<SyncQuality>,
    pub audio_format: Option<AudioFormat>,
    pub error: Option<String>,

    pub should_quit: bool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            title: None,
            artist: None,
            album: None,
            track_progress_ms: 0,
            track_duration_ms: 0,
            playback_speed: 0,
            progress_updated_at: Instant::now(),
            volume: 100,
            muted: false,
            repeat: RepeatMode::Off,
            shuffle: false,
            playback_state: PlaybackState::Stopped,
            group_name: None,
            connected: false,
            device_name: None,
            last_data_at: None,
            sync_rtt_ms: None,
            sync_quality: None,
            audio_format: None,
            error: None,
            should_quit: false,
        }
    }

    /// Current interpolated progress in milliseconds.
    pub fn current_progress_ms(&self) -> i64 {
        if self.playback_speed == 0 {
            return self.track_progress_ms;
        }
        let elapsed = self.progress_updated_at.elapsed().as_millis() as i64;
        let delta = elapsed * self.playback_speed as i64 / 1000;
        let progress = self.track_progress_ms + delta;
        if self.track_duration_ms > 0 {
            progress.min(self.track_duration_ms)
        } else {
            progress
        }
    }

    /// Progress ratio 0.0..1.0 for the gauge widget.
    pub fn progress_ratio(&self) -> f64 {
        if self.track_duration_ms <= 0 {
            return 0.0;
        }
        (self.current_progress_ms() as f64 / self.track_duration_ms as f64).clamp(0.0, 1.0)
    }

    /// Whether audio is currently playing (playback speed > 0).
    pub fn is_playing(&self) -> bool {
        self.playback_speed > 0
    }

    /// Whether we appear to have lost connection (no data for >10s while connected).
    pub fn is_stale(&self) -> bool {
        if !self.connected {
            return false;
        }
        match self.last_data_at {
            Some(t) => t.elapsed() > std::time::Duration::from_secs(10),
            None => false,
        }
    }

    /// Apply a protocol event to update the application state.
    pub fn handle_event(&mut self, event: AppEvent) {
        match event {
            AppEvent::Connected { device_name } => {
                self.connected = true;
                self.device_name = device_name;
                self.last_data_at = Some(Instant::now());
                self.error = None;
            }
            AppEvent::Disconnected(reason) => {
                self.connected = false;
                self.last_data_at = None;
                self.error = Some(reason);
            }
            AppEvent::Metadata(meta) => {
                self.last_data_at = Some(Instant::now());
                if meta.title.is_some() {
                    self.title = meta.title;
                }
                if meta.artist.is_some() {
                    self.artist = meta.artist;
                }
                if meta.album.is_some() {
                    self.album = meta.album;
                }
                if let Some(progress) = meta.progress {
                    self.track_progress_ms = progress.track_progress;
                    self.track_duration_ms = progress.track_duration;
                    self.playback_speed = progress.playback_speed;
                    self.progress_updated_at = Instant::now();
                }
                if let Some(repeat) = meta.repeat {
                    self.repeat = repeat;
                }
                if let Some(shuffle) = meta.shuffle {
                    self.shuffle = shuffle;
                }
            }
            AppEvent::Controller(ctrl) => {
                self.volume = ctrl.volume;
                self.muted = ctrl.muted;
            }
            AppEvent::LocalStateUpdate {
                volume,
                muted,
                playing,
                repeat,
                shuffle,
            } => {
                self.volume = volume;
                self.muted = muted;
                // Update playback speed without resetting progress interpolation
                let new_speed = if playing { 1000 } else { 0 };
                if new_speed != self.playback_speed {
                    // Anchor progress at current interpolated position before changing speed
                    self.track_progress_ms = self.current_progress_ms();
                    self.progress_updated_at = std::time::Instant::now();
                    self.playback_speed = new_speed;
                }
                self.repeat = repeat;
                self.shuffle = shuffle;
            }
            AppEvent::GroupUpdate {
                playback_state,
                group_name,
            } => {
                if let Some(state) = playback_state {
                    self.playback_state = state;
                }
                if group_name.is_some() {
                    self.group_name = group_name;
                }
            }
            AppEvent::StreamStarted(format) => {
                self.audio_format = Some(format);
            }
            AppEvent::StreamEnded => {
                self.playback_speed = 0;
                self.progress_updated_at = Instant::now();
            }
            AppEvent::ClockSync { rtt_ms, quality } => {
                self.last_data_at = Some(Instant::now());
                self.sync_rtt_ms = Some(rtt_ms);
                self.sync_quality = Some(quality);
            }
            AppEvent::Error(msg) => {
                self.error = Some(msg);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sendspin::audio::Codec;

    fn metadata_with_title(title: &str) -> MetadataState {
        MetadataState {
            timestamp: 0,
            title: Some(title.to_string()),
            artist: None,
            album_artist: None,
            album: None,
            artwork_url: None,
            year: None,
            track: None,
            progress: None,
            repeat: None,
            shuffle: None,
        }
    }

    fn metadata_with_progress(progress_ms: i64, duration_ms: i64, speed: i32) -> MetadataState {
        MetadataState {
            timestamp: 0,
            title: None,
            artist: None,
            album_artist: None,
            album: None,
            artwork_url: None,
            year: None,
            track: None,
            progress: Some(TrackProgress {
                track_progress: progress_ms,
                track_duration: duration_ms,
                playback_speed: speed,
            }),
            repeat: None,
            shuffle: None,
        }
    }

    #[test]
    fn new_state_defaults() {
        let state = AppState::new();
        assert!(!state.connected);
        assert!(!state.is_playing());
        assert_eq!(state.volume, 100);
        assert!(!state.muted);
        assert!(matches!(state.repeat, RepeatMode::Off));
        assert!(!state.shuffle);
        assert!(state.title.is_none());
        assert_eq!(state.progress_ratio(), 0.0);
    }

    #[test]
    fn connected_clears_error() {
        let mut state = AppState::new();
        state.error = Some("old error".into());
        state.handle_event(AppEvent::Connected { device_name: None });
        assert!(state.connected);
        assert!(state.error.is_none());
    }

    #[test]
    fn disconnected_sets_error() {
        let mut state = AppState::new();
        state.connected = true;
        state.handle_event(AppEvent::Disconnected("connection lost".into()));
        assert!(!state.connected);
        assert_eq!(state.error.as_deref(), Some("connection lost"));
    }

    #[test]
    fn metadata_updates_title() {
        let mut state = AppState::new();
        state.handle_event(AppEvent::Metadata(metadata_with_title("Test Song")));
        assert_eq!(state.title.as_deref(), Some("Test Song"));
    }

    #[test]
    fn metadata_none_fields_dont_overwrite() {
        let mut state = AppState::new();
        state.title = Some("Original".into());
        state.artist = Some("Artist".into());

        // Send metadata with only title set — artist should remain
        state.handle_event(AppEvent::Metadata(metadata_with_title("New Title")));
        assert_eq!(state.title.as_deref(), Some("New Title"));
        assert_eq!(state.artist.as_deref(), Some("Artist"));
    }

    #[test]
    fn metadata_updates_progress() {
        let mut state = AppState::new();
        state.handle_event(AppEvent::Metadata(metadata_with_progress(
            30_000, 180_000, 1000,
        )));
        assert_eq!(state.track_progress_ms, 30_000);
        assert_eq!(state.track_duration_ms, 180_000);
        assert_eq!(state.playback_speed, 1000);
        assert!(state.is_playing());
    }

    #[test]
    fn controller_updates_volume_and_mute() {
        let mut state = AppState::new();
        state.handle_event(AppEvent::Controller(ControllerState {
            supported_commands: vec![],
            volume: 75,
            muted: true,
        }));
        assert_eq!(state.volume, 75);
        assert!(state.muted);
    }

    #[test]
    fn progress_ratio_zero_when_no_duration() {
        let mut state = AppState::new();
        state.track_progress_ms = 5000;
        state.track_duration_ms = 0;
        assert_eq!(state.progress_ratio(), 0.0);
    }

    #[test]
    fn progress_ratio_clamped_to_one() {
        let mut state = AppState::new();
        state.track_progress_ms = 200_000;
        state.track_duration_ms = 180_000;
        state.playback_speed = 0; // paused so interpolation doesn't add more
        assert_eq!(state.progress_ratio(), 1.0);
    }

    #[test]
    fn progress_paused_does_not_advance() {
        let mut state = AppState::new();
        state.track_progress_ms = 60_000;
        state.track_duration_ms = 180_000;
        state.playback_speed = 0;
        state.progress_updated_at = Instant::now() - std::time::Duration::from_secs(10);
        // Even though 10 seconds elapsed, paused should not advance
        assert_eq!(state.current_progress_ms(), 60_000);
    }

    #[test]
    fn progress_capped_at_duration() {
        let mut state = AppState::new();
        state.track_progress_ms = 179_000;
        state.track_duration_ms = 180_000;
        state.playback_speed = 1000;
        // Set updated_at 5 seconds ago — would overshoot 180s
        state.progress_updated_at = Instant::now() - std::time::Duration::from_secs(5);
        assert_eq!(state.current_progress_ms(), 180_000);
    }

    #[test]
    fn local_state_update_volume_preserves_progress() {
        let mut state = AppState::new();
        state.track_progress_ms = 60_000;
        state.track_duration_ms = 180_000;
        state.playback_speed = 1000;
        state.progress_updated_at = Instant::now();

        // Volume change should not reset progress
        state.handle_event(AppEvent::LocalStateUpdate {
            volume: 80,
            muted: false,
            playing: true, // same as current
            repeat: RepeatMode::Off,
            shuffle: false,
        });
        assert_eq!(state.volume, 80);
        // Progress should be >= 60_000 (not reset to 0)
        assert!(state.current_progress_ms() >= 60_000);
    }

    #[test]
    fn local_state_update_pause_anchors_progress() {
        let mut state = AppState::new();
        state.track_progress_ms = 60_000;
        state.track_duration_ms = 180_000;
        state.playback_speed = 1000;
        state.progress_updated_at = Instant::now();

        state.handle_event(AppEvent::LocalStateUpdate {
            volume: 100,
            muted: false,
            playing: false, // pause
            repeat: RepeatMode::Off,
            shuffle: false,
        });
        assert_eq!(state.playback_speed, 0);
        // Progress should be anchored at ~60s, not reset to 0
        assert!(state.current_progress_ms() >= 60_000);
    }

    #[test]
    fn stream_ended_stops_playback() {
        let mut state = AppState::new();
        state.playback_speed = 1000;
        state.handle_event(AppEvent::StreamEnded);
        assert_eq!(state.playback_speed, 0);
        assert!(!state.is_playing());
    }

    #[test]
    fn stream_started_sets_format() {
        let mut state = AppState::new();
        let fmt = AudioFormat {
            codec: Codec::Pcm,
            sample_rate: 48000,
            channels: 2,
            bit_depth: 24,
            codec_header: None,
        };
        state.handle_event(AppEvent::StreamStarted(fmt));
        let f = state.audio_format.as_ref().unwrap();
        assert_eq!(f.sample_rate, 48000);
        assert_eq!(f.bit_depth, 24);
    }

    #[test]
    fn clock_sync_updates() {
        let mut state = AppState::new();
        state.handle_event(AppEvent::ClockSync {
            rtt_ms: 12.5,
            quality: SyncQuality::Good,
        });
        assert_eq!(state.sync_rtt_ms, Some(12.5));
        assert!(matches!(state.sync_quality, Some(SyncQuality::Good)));
    }

    #[test]
    fn group_update_partial() {
        let mut state = AppState::new();
        state.group_name = Some("Room 1".into());
        // Update only playback_state, group_name is None — should not overwrite
        state.handle_event(AppEvent::GroupUpdate {
            playback_state: Some(PlaybackState::Playing),
            group_name: None,
        });
        assert!(matches!(state.playback_state, PlaybackState::Playing));
        assert_eq!(state.group_name.as_deref(), Some("Room 1"));
    }
}
