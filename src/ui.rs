use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Gauge, List, ListItem, ListState, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use std::ffi::OsStr;
use std::io;
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::Duration;

use crate::types::{Event, EventPayload, OnRepeat, Status, TrackData, UiMessage};

fn fmt_dur(d: Duration) -> String {
    format!("{}:{:02}", d.as_secs() / 60, d.as_secs() % 60)
}

fn label(t: &TrackData) -> String {
    match &t.tag {
        Some(tag) if tag.title != "Unknown Title" => format!("{} — {}", tag.artist, tag.title),
        _ => Path::new(&t.path)
            .file_stem()
            .and_then(OsStr::to_str)
            .unwrap_or("?")
            .to_string(),
    }
}

pub struct Ui {
    pub tx: Sender<Event>,
    rx: Receiver<UiMessage>,
    library: Vec<TrackData>,
    status: Status,
    list: ListState,
    message: Option<String>,
    pub fatal: Option<String>,
    running: bool,
}

impl Ui {
    pub fn new(tx: Sender<Event>, rx: Receiver<UiMessage>) -> Self {
        Self {
            tx,
            rx,
            library: Vec::new(),
            status: Status {
                current: None,
                position: Duration::ZERO,
                paused: true,
                volume: 1.0,
                on_repeat: OnRepeat::Yes,
                shuffle: true,
            },
            list: ListState::default(),
            message: Some("Scanning library…".into()),
            fatal: None,
            running: true,
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        while self.running {
            self.drain();
            terminal.draw(|f| self.draw(f))?;
            if event::poll(Duration::from_millis(50))? {
                if let TermEvent::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        self.on_key(key.code, key.modifiers);
                    }
                }
            }
        }
        Ok(())
    }
    fn drain(&mut self) {
        loop {
            match self.rx.try_recv() {
                Ok(UiMessage::Library(mut tracks)) => {
                    tracks.sort_by_cached_key(|t| label(t).to_lowercase());
                    self.list
                        .select(if tracks.is_empty() { None } else { Some(0) });
                    self.library = tracks;
                    self.message = None;
                }
                Ok(UiMessage::Status(s)) => self.status = s,
                Ok(UiMessage::Notice(m)) => self.message = Some(m),
                Ok(UiMessage::Fatal(m)) => {
                    self.fatal = Some(m);
                    self.running = false;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.running = false;
                    break;
                }
            }
        }
    }

    fn send(&mut self, payload: EventPayload) {
        if self.tx.send(Event { payload }).is_err() {
            self.running = false;
        }
    }

    fn on_key(&mut self, code: KeyCode, mods: KeyModifiers) {
        self.message = None;
        let selected = self
            .list
            .selected()
            .and_then(|i| self.library.get(i))
            .map(|t| t.id);

        match code {
            KeyCode::Char('c') if mods.contains(KeyModifiers::CONTROL) => self.quit(),
            KeyCode::Char('q') | KeyCode::Esc => self.quit(),
            KeyCode::Up | KeyCode::Char('k') => self.list.select_previous(),
            KeyCode::Down | KeyCode::Char('j') => self.list.select_next(),
            KeyCode::Enter => {
                if let Some(id) = selected {
                    self.send(EventPayload::PlaySong { id });
                }
            }
            KeyCode::Char('e') => {
                if let Some(id) = selected {
                    self.send(EventPayload::Enqueue { id });
                }
            }
            KeyCode::Char(' ') => self.send(EventPayload::TogglePause),
            KeyCode::Char('n') => self.send(EventPayload::NextSong),
            KeyCode::Char('p') => self.send(EventPayload::PreviousSong),
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.send(EventPayload::ChangeVolume { value: 0.05 })
            }
            KeyCode::Char('-') => self.send(EventPayload::ChangeVolume { value: -0.05 }),
            KeyCode::Char('s') => self.send(EventPayload::Shuffle),
            KeyCode::Char('r') => {
                let mode = match self.status.on_repeat {
                    OnRepeat::Yes => OnRepeat::Single,
                    OnRepeat::Single => OnRepeat::No,
                    OnRepeat::No => OnRepeat::Yes,
                };
                self.send(EventPayload::ChangeRepeatMode { mode });
            }
            _ => {}
        }
    }

    fn quit(&mut self) {
        self.send(EventPayload::Exit);
        self.running = false;
    }

    fn draw(&mut self, f: &mut Frame) {
        let [list_area, now_area, help_area] = Layout::vertical([
            Constraint::Min(3),
            Constraint::Length(4),
            Constraint::Length(1),
        ])
        .areas(f.area());

        let items: Vec<ListItem> = self
            .library
            .iter()
            .map(|t| {
                let playing = Some(t.id) == self.status.current;
                let dur = t.duration.map_or("--:--".to_string(), fmt_dur);
                let style = if playing {
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                ListItem::new(format!(
                    "{} {:>4}  {}  [{}]",
                    if playing { "▶" } else { " " },
                    t.id,
                    label(t),
                    dur
                ))
                .style(style)
            })
            .collect();
        let list = List::new(items)
            .block(Block::bordered().title(format!(" Library ({}) ", self.library.len())))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        f.render_stateful_widget(list, list_area, &mut self.list);

        let block = Block::bordered().title(" Now playing ");
        let inner = block.inner(now_area);
        f.render_widget(block, now_area);
        let [info, bar] = Layout::vertical([Constraint::Length(1); 2]).areas(inner);

        let current = self
            .library
            .iter()
            .find(|t| Some(t.id) == self.status.current);
        let name = current.map_or("—".to_string(), label);
        let dur = current.and_then(|t| t.duration);
        let icon = match (current, self.status.paused) {
            (None, _) => "■",
            (_, true) => "⏸",
            (_, false) => "▶",
        };
        let mode = match self.status.on_repeat {
            OnRepeat::Yes => "repeat",
            OnRepeat::No => "",
            OnRepeat::Single => "repeat single",
        };
        f.render_widget(
            Paragraph::new(format!(
                "{icon} {name}    vol {}%   {mode}",
                (self.status.volume * 100.0).round() as u32
            )),
            info,
        );

        let ratio = dur.filter(|d| !d.is_zero()).map_or(0.0, |d| {
            (self.status.position.as_secs_f64() / d.as_secs_f64()).clamp(0.0, 1.0)
        });
        let text = format!(
            "{} / {}",
            fmt_dur(self.status.position),
            dur.map_or("--:--".to_string(), fmt_dur)
        );
        f.render_widget(
            Gauge::default()
                .gauge_style(Style::default().fg(Color::Cyan))
                .ratio(ratio)
                .label(text),
            bar,
        );
        let (text, color) = match &self.message {
            Some(m) => (m.as_str(), Color::Red),
            None => (
                "↑↓ move  Enter play  e enqueue  Space pause  n/p next/prev  +/- vol  s shuffle  r repeat  q quit",
                Color::DarkGray,
            ),
        };
        f.render_widget(
            Paragraph::new(text).style(Style::default().fg(color)),
            help_area,
        );
    }
}
