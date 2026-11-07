use std::error::Error;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

pub mod player;
pub mod resources;
pub mod server;
pub mod types;
pub mod ui;

use crate::server::backend_main;
use crate::types::{Event, EventPayload, UiMessage};
use crate::ui::Ui;

fn main() -> Result<(), Box<dyn Error>> {
    let dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let (cmd_tx, cmd_rx) = mpsc::channel::<Event>();
    let (ui_tx, ui_rx) = mpsc::channel::<UiMessage>();
    let backend = thread::spawn(move || backend_main(dir, cmd_rx, ui_tx));

    let mut ui = Ui::new(cmd_tx, ui_rx);
    let mut terminal = ratatui::init();
    let result = ui.run(&mut terminal);
    ratatui::restore();
    let _ = ui.tx.send(Event {
        payload: EventPayload::Exit,
    });
    let panicked = backend.join().is_err();

    result?;
    if let Some(msg) = ui.fatal {
        return Err(msg.into());
    }
    if panicked {
        return Err("backend thread panicked".into());
    }
    Ok(())
}
