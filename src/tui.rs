use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::DefaultTerminal;
use tokio::sync::mpsc;

use crate::app::AppState;
use crate::event::{AppEvent, Command};
use crate::ui;

/// Run the TUI event loop. Blocks until the user quits.
pub fn run(
    mut terminal: DefaultTerminal,
    mut event_rx: mpsc::UnboundedReceiver<AppEvent>,
    command_tx: mpsc::UnboundedSender<Command>,
) -> color_eyre::Result<()> {
    let mut state = AppState::new();

    loop {
        // Drain all pending protocol events (non-blocking)
        while let Ok(event) = event_rx.try_recv() {
            state.handle_event(event);
        }

        // Render
        terminal.draw(|frame| ui::draw(frame, &state))?;

        if state.should_quit {
            break;
        }

        // Poll for keyboard events with timeout (~20fps)
        if event::poll(Duration::from_millis(50))?
            && let Event::Key(key) = event::read()?
        {
                // Only handle key press events (not release/repeat)
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        let _ = command_tx.send(Command::Quit);
                        state.should_quit = true;
                    }
                    KeyCode::Char(' ') => {
                        let _ = command_tx.send(Command::PlayPause);
                    }
                    KeyCode::Char('n') | KeyCode::Right => {
                        let _ = command_tx.send(Command::Next);
                    }
                    KeyCode::Char('p') | KeyCode::Left => {
                        let _ = command_tx.send(Command::Previous);
                    }
                    KeyCode::Up | KeyCode::Char('+') | KeyCode::Char('=') => {
                        let _ = command_tx.send(Command::VolumeUp);
                    }
                    KeyCode::Down | KeyCode::Char('-') => {
                        let _ = command_tx.send(Command::VolumeDown);
                    }
                    KeyCode::Char('m') => {
                        let _ = command_tx.send(Command::Mute);
                    }
                    KeyCode::Char('r') => {
                        let _ = command_tx.send(Command::CycleRepeat);
                    }
                    KeyCode::Char('s') => {
                        let _ = command_tx.send(Command::ToggleShuffle);
                    }
                    _ => {}
                }
        }
    }

    Ok(())
}
