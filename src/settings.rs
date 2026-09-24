use crate::{
    api::ApiClient,
    config::Config,
    theme::Palette,
    ui::{Action, App},
};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Modifier, Style},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use std::path::Path;

pub struct Settings {
    pub config: Config,
    pub username: String,
    pub password: String,
    pub token: String,
    pub field: usize,
    pub busy: bool,
    pub message: String,
    pub devices: Vec<(String, String)>,
}
impl Settings {
    pub fn paste(&mut self, text: &str) {
        if self.busy {
            return;
        }
        let value = match self.field {
            0 => &mut self.config.server,
            1 => &mut self.username,
            2 => &mut self.password,
            3 => &mut self.token,
            4 => &mut self.config.player_name,
            _ => return,
        };
        if !crate::ui::append_paste(value, text, 4096) {
            self.message = "Paste exceeds this field's 4096-byte limit; nothing inserted".into();
        }
    }

    pub fn new(config: Config) -> Self {
        Self {
            config,
            username: String::new(),
            password: String::new(),
            token: String::new(),
            field: 0,
            busy: false,
            message: "Enter server and a profile token, OR a built-in username/password.".into(),
            devices: crate::audio::devices()
                .unwrap_or_default()
                .into_iter()
                .map(|d| (d.id, d.name))
                .collect(),
        }
    }
    pub fn key(&mut self, key: KeyEvent) -> Action {
        if key.code == KeyCode::Esc {
            return Action::Quit;
        }
        if self.busy {
            return Action::None;
        }
        match key.code {
            KeyCode::Tab | KeyCode::Down => self.field = (self.field + 1) % 8,
            KeyCode::BackTab | KeyCode::Up => self.field = (self.field + 7) % 8,
            KeyCode::Enter if self.field == 7 => return Action::SaveSettings,
            KeyCode::Enter => self.field = (self.field + 1) % 8,
            KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right if self.field == 5 => {
                self.config.local_playback = !self.config.local_playback
            }
            KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right if self.field == 6 => {
                let index = self
                    .config
                    .device_id
                    .as_ref()
                    .and_then(|id| self.devices.iter().position(|d| &d.0 == id));
                self.config.device_id = match index {
                    None => self.devices.first().map(|d| d.0.clone()),
                    Some(i) => self.devices.get(i + 1).map(|d| d.0.clone()),
                };
            }
            _ => {
                let value = match self.field {
                    0 => &mut self.config.server,
                    1 => &mut self.username,
                    2 => &mut self.password,
                    3 => &mut self.token,
                    4 => &mut self.config.player_name,
                    _ => return Action::None,
                };
                match key.code {
                    KeyCode::Backspace => {
                        value.pop();
                    }
                    KeyCode::Char('u')
                        if key
                            .modifiers
                            .contains(crossterm::event::KeyModifiers::CONTROL) =>
                    {
                        value.clear()
                    }
                    KeyCode::Char(c) if !c.is_control() && value.len() < 4096 => value.push(c),
                    _ => {}
                }
            }
        }
        Action::None
    }
    pub fn draw(&self, frame: &mut Frame, palette: Palette) {
        let area = frame.area();
        frame.render_widget(
            Block::default().style(
                Style::default()
                    .bg(palette.background)
                    .fg(palette.foreground),
            ),
            area,
        );
        let rows = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(8),
            Constraint::Min(1),
        ])
        .margin(1)
        .split(area);
        frame.render_widget(Paragraph::new(concat!("MA-TUI v", env!("CARGO_PKG_VERSION"), " · CONNECTION SETTINGS\nTab/Shift-Tab fields · Ctrl-U clear · Space toggles · Esc cancel")), rows[0]);
        let device = self
            .config
            .device_id
            .as_ref()
            .map(|id| {
                self.devices
                    .iter()
                    .find(|d| &d.0 == id)
                    .map(|d| d.1.as_str())
                    .unwrap_or("Saved device (currently unavailable)")
            })
            .unwrap_or("System default");
        let fields = [
            format!("Server URL: {}", self.config.server),
            format!("Username: {}", self.username),
            format!(
                "Password: {}",
                "•".repeat(self.password.chars().count().min(40))
            ),
            format!(
                "Profile token: {}",
                "•".repeat(self.token.chars().count().min(40))
            ),
            format!("Speaker name: {}", self.config.player_name),
            format!(
                "Expose this computer as a speaker: {}",
                if self.config.local_playback {
                    "Yes"
                } else {
                    "No"
                }
            ),
            format!("Audio output: {device}"),
            "[ Test connection and save login ]".to_owned(),
        ];
        for (i, row) in Layout::vertical([Constraint::Length(1); 8])
            .split(rows[1])
            .iter()
            .enumerate()
        {
            let style = if self.field == i {
                Style::default()
                    .fg(palette.accent)
                    .bg(palette.selection)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            frame.render_widget(
                Paragraph::new(format!(
                    "{} {}",
                    if self.field == i { "›" } else { " " },
                    fields[i]
                ))
                .style(style),
                *row,
            );
        }
        frame.render_widget(Paragraph::new(format!("{}\n\nTokens are saved in the desktop keyring. Passwords are never saved.\nBlank credentials reuse the saved login for the same server.\nHTTP sends credentials without encryption; use HTTPS outside a trusted LAN.\nEnabling the speaker lets Music Assistant send audio while MA-TUI is open.", self.message))
            .wrap(ratatui::widgets::Wrap { trim: false }).block(Block::default().borders(Borders::TOP)), rows[2]);
    }
}

/// A separate screen keeps editing credentials isolated from playback shortcuts.
pub fn run(
    config: Config,
    existing: Option<String>,
    path: &Path,
) -> Result<Option<(Config, String)>> {
    let previous_server = config.server.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let mut completed = None;
    let mut task: Option<tokio::task::JoinHandle<()>> = None;
    let result = crate::terminal_ui::run(
        App {
            settings: Some(Settings::new(config)),
            ..App::default()
        },
        |app| {
            let Ok(result) = rx.try_recv() else {
                return false;
            };
            match result {
                Ok(connection) => {
                    completed = Some(connection);
                    app.exit = true;
                }
                Err(message) => {
                    let s = app.settings.as_mut().unwrap();
                    s.busy = false;
                    s.message = message;
                }
            }
            true
        },
        |app, action| {
            if action != Action::SaveSettings {
                return;
            }
            let settings = app.settings.as_mut().unwrap();
            let config = settings.config.clone();
            // Keep the form intact while testing. A failed request must remain
            // retryable without re-entering a long token; rendering stays masked.
            let username = settings.username.clone();
            let password = settings.password.clone();
            let entered = settings.token.clone();
            let saved = if config.server == previous_server {
                existing.clone()
            } else {
                None
            };
            let path = path.to_owned();
            let tx = tx.clone();
            settings.busy = true;
            settings.message = "Testing login and saving to the desktop keyring…".into();
            task = Some(tokio::spawn(async move {
                let result = async {
                    let config = Config::parse(&toml::to_string(&config)?)?;
                    let token = if !entered.is_empty() {
                        entered
                    } else if !username.is_empty() || !password.is_empty() {
                        ApiClient::login(&config.server, &username, &password).await?
                    } else if let Some(token) = saved {
                        token
                    } else {
                        crate::credentials::load(&config.server, &config.player_id).await?
                    };
                    let api = ApiClient::new(&config.server, &token)?;
                    api.verify().await?;
                    crate::credentials::save(&config.server, &config.player_id, &token).await?;
                    config.save(&path)?;
                    Ok::<_, anyhow::Error>((config, token))
                }
                .await;
                let _ = tx.send(result.map_err(|e| e.to_string()));
            }));
        },
    );
    if let Some(task) = task {
        task.abort();
    }
    result?;
    Ok(completed)
}
