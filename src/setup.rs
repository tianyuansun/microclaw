use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

use chrono::Utc;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::layout::{Constraint, Direction, Layout, Margin};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::DefaultTerminal;

use crate::error::MicroClawError;

const ENV_KEYS: &[&str] = &[
    "TELEGRAM_BOT_TOKEN",
    "BOT_USERNAME",
    "LLM_PROVIDER",
    "LLM_API_KEY",
    "LLM_MODEL",
    "LLM_BASE_URL",
    "DATA_DIR",
    "TIMEZONE",
];

#[derive(Clone)]
struct Field {
    key: &'static str,
    label: &'static str,
    value: String,
    required: bool,
    secret: bool,
}

impl Field {
    fn display_value(&self, editing: bool) -> String {
        if editing || !self.secret {
            return self.value.clone();
        }
        if self.value.is_empty() {
            String::new()
        } else {
            mask_secret(&self.value)
        }
    }
}

struct SetupApp {
    fields: Vec<Field>,
    selected: usize,
    editing: bool,
    status: String,
    completed: bool,
    backup_path: Option<String>,
    completion_summary: Vec<String>,
}

impl SetupApp {
    fn new() -> Self {
        dotenvy::dotenv().ok();
        let provider = std::env::var("LLM_PROVIDER").unwrap_or_else(|_| "anthropic".into());
        let default_model = if provider == "openai" {
            "gpt-4o"
        } else {
            "claude-sonnet-4-20250514"
        };
        let llm_api_key = std::env::var("LLM_API_KEY")
            .or_else(|_| std::env::var("ANTHROPIC_API_KEY"))
            .unwrap_or_default();

        Self {
            fields: vec![
                Field {
                    key: "TELEGRAM_BOT_TOKEN",
                    label: "Telegram bot token",
                    value: std::env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default(),
                    required: true,
                    secret: true,
                },
                Field {
                    key: "BOT_USERNAME",
                    label: "Bot username (without @)",
                    value: std::env::var("BOT_USERNAME").unwrap_or_default(),
                    required: true,
                    secret: false,
                },
                Field {
                    key: "LLM_PROVIDER",
                    label: "LLM provider (anthropic/openai)",
                    value: provider,
                    required: true,
                    secret: false,
                },
                Field {
                    key: "LLM_API_KEY",
                    label: "LLM API key",
                    value: llm_api_key,
                    required: true,
                    secret: true,
                },
                Field {
                    key: "LLM_MODEL",
                    label: "LLM model",
                    value: std::env::var("LLM_MODEL")
                        .or_else(|_| std::env::var("CLAUDE_MODEL"))
                        .unwrap_or_else(|_| default_model.into()),
                    required: false,
                    secret: false,
                },
                Field {
                    key: "LLM_BASE_URL",
                    label: "LLM base URL (optional)",
                    value: std::env::var("LLM_BASE_URL").unwrap_or_default(),
                    required: false,
                    secret: false,
                },
                Field {
                    key: "DATA_DIR",
                    label: "Data directory",
                    value: std::env::var("DATA_DIR").unwrap_or_else(|_| "./data".into()),
                    required: false,
                    secret: false,
                },
                Field {
                    key: "TIMEZONE",
                    label: "Timezone (IANA)",
                    value: std::env::var("TIMEZONE").unwrap_or_else(|_| "UTC".into()),
                    required: false,
                    secret: false,
                },
            ],
            selected: 0,
            editing: false,
            status: "Use ↑/↓ select field, Enter edit, F2 validate, s/Ctrl+S save, q quit".into(),
            completed: false,
            backup_path: None,
            completion_summary: Vec::new(),
        }
    }

    fn next(&mut self) {
        if self.selected + 1 < self.fields.len() {
            self.selected += 1;
        }
    }

    fn prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    fn selected_field_mut(&mut self) -> &mut Field {
        &mut self.fields[self.selected]
    }

    fn selected_field(&self) -> &Field {
        &self.fields[self.selected]
    }

    fn field_value(&self, key: &str) -> String {
        self.fields
            .iter()
            .find(|f| f.key == key)
            .map(|f| f.value.trim().to_string())
            .unwrap_or_default()
    }

    fn to_env_map(&self) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for field in &self.fields {
            if !field.value.trim().is_empty() {
                out.insert(field.key.to_string(), field.value.trim().to_string());
            }
        }
        out
    }

    fn validate_local(&self) -> Result<(), MicroClawError> {
        for field in &self.fields {
            if field.required && field.value.trim().is_empty() {
                return Err(MicroClawError::Config(format!("{} is required", field.key)));
            }
        }

        let provider = self.field_value("LLM_PROVIDER").to_lowercase();
        if provider != "anthropic" && provider != "openai" {
            return Err(MicroClawError::Config(
                "LLM_PROVIDER must be 'anthropic' or 'openai'".into(),
            ));
        }

        let username = self.field_value("BOT_USERNAME");
        if username.starts_with('@') {
            return Err(MicroClawError::Config(
                "BOT_USERNAME should not include '@'".into(),
            ));
        }

        let timezone = self.field_value("TIMEZONE");
        let tz = if timezone.is_empty() {
            "UTC".to_string()
        } else {
            timezone
        };
        tz.parse::<chrono_tz::Tz>()
            .map_err(|_| MicroClawError::Config(format!("Invalid TIMEZONE: {tz}")))?;

        let data_dir = self.field_value("DATA_DIR");
        let dir = if data_dir.is_empty() {
            "./data".to_string()
        } else {
            data_dir
        };
        fs::create_dir_all(&dir)?;
        let probe = Path::new(&dir).join(".setup_probe");
        fs::write(&probe, "ok")?;
        let _ = fs::remove_file(probe);

        Ok(())
    }

    fn validate_online(&self) -> Result<Vec<String>, MicroClawError> {
        let tg_token = self.field_value("TELEGRAM_BOT_TOKEN");
        let env_username = self
            .field_value("BOT_USERNAME")
            .trim_start_matches('@')
            .to_string();
        let provider = self.field_value("LLM_PROVIDER").to_lowercase();
        let api_key = self.field_value("LLM_API_KEY");
        let base_url = self.field_value("LLM_BASE_URL");
        std::thread::spawn(move || {
            perform_online_validation(&tg_token, &env_username, &provider, &api_key, &base_url)
        })
        .join()
        .map_err(|_| MicroClawError::Config("Validation thread panicked".into()))?
    }

    fn set_provider(&mut self, provider: &str) {
        if let Some(field) = self.fields.iter_mut().find(|f| f.key == "LLM_PROVIDER") {
            field.value = provider.to_string();
        }
        if let Some(model) = self.fields.iter_mut().find(|f| f.key == "LLM_MODEL") {
            if model.value.trim().is_empty()
                || model.value == "gpt-4o"
                || model.value == "claude-sonnet-4-20250514"
            {
                model.value = if provider == "openai" {
                    "gpt-4o".into()
                } else {
                    "claude-sonnet-4-20250514".into()
                };
            }
        }
    }

    fn current_section(&self) -> &'static str {
        match self.selected {
            0..=1 => "Telegram",
            2..=5 => "LLM",
            6..=7 => "Runtime",
            _ => "Setup",
        }
    }

    fn progress_bar(&self, width: usize) -> String {
        let total = self.fields.len().max(1);
        let done = self.selected + 1;
        let fill = (done * width) / total;
        let mut s = String::new();
        for i in 0..width {
            if i < fill {
                s.push('█');
            } else {
                s.push('░');
            }
        }
        s
    }
}

fn perform_online_validation(
    tg_token: &str,
    env_username: &str,
    provider: &str,
    api_key: &str,
    base_url: &str,
) -> Result<Vec<String>, MicroClawError> {
    let mut checks = Vec::new();
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()?;

    let tg_resp: serde_json::Value = client
        .get(format!("https://api.telegram.org/bot{tg_token}/getMe"))
        .send()?
        .json()?;
    let ok = tg_resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    if !ok {
        return Err(MicroClawError::Config(
            "Telegram getMe failed (check TELEGRAM_BOT_TOKEN)".into(),
        ));
    }
    let actual_username = tg_resp
        .get("result")
        .and_then(|r| r.get("username"))
        .and_then(|u| u.as_str())
        .unwrap_or_default()
        .to_string();
    if !env_username.is_empty() && !actual_username.is_empty() && env_username != actual_username {
        checks.push(format!(
            "Telegram OK (token user={actual_username}, configured={env_username})"
        ));
    } else {
        checks.push(format!("Telegram OK ({actual_username})"));
    }

    if provider == "openai" {
        let base = if base_url.is_empty() {
            "https://api.openai.com".to_string()
        } else {
            base_url.trim_end_matches('/').to_string()
        };
        let status = client
            .get(format!("{base}/v1/models"))
            .bearer_auth(api_key)
            .send()?
            .status();
        if !status.is_success() {
            return Err(MicroClawError::Config(format!(
                "OpenAI validation failed: HTTP {status}"
            )));
        }
        checks.push("LLM OK (openai)".into());
    } else {
        let base = if base_url.is_empty() {
            "https://api.anthropic.com".to_string()
        } else {
            base_url.trim_end_matches('/').to_string()
        };
        let status = client
            .get(format!("{base}/v1/models"))
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .send()?
            .status();
        if !status.is_success() {
            return Err(MicroClawError::Config(format!(
                "Anthropic validation failed: HTTP {status}"
            )));
        }
        checks.push("LLM OK (anthropic)".into());
    }

    Ok(checks)
}

fn mask_secret(s: &str) -> String {
    if s.len() <= 6 {
        return "***".into();
    }
    format!("{}***{}", &s[..3], &s[s.len() - 2..])
}

fn save_env_file(
    path: &Path,
    values: &HashMap<String, String>,
) -> Result<Option<String>, MicroClawError> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    let mut backup = None;
    if path.exists() {
        let ts = Utc::now().format("%Y%m%d%H%M%S").to_string();
        let backup_path = format!("{}.bak.{ts}", path.display());
        fs::copy(path, &backup_path)?;
        backup = Some(backup_path);
    }

    let mut new_lines = Vec::new();
    let mut seen = HashMap::<String, bool>::new();
    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || !trimmed.contains('=') {
            new_lines.push(line.to_string());
            continue;
        }
        let key = trimmed
            .split_once('=')
            .map(|(k, _)| k.trim().to_string())
            .unwrap_or_default();
        if ENV_KEYS.contains(&key.as_str()) {
            if let Some(v) = values.get(&key) {
                new_lines.push(format!("{key}={v}"));
                seen.insert(key, true);
            } else {
                new_lines.push(line.to_string());
            }
        } else {
            new_lines.push(line.to_string());
        }
    }

    for key in ENV_KEYS {
        if !seen.contains_key(*key) {
            if let Some(v) = values.get(*key) {
                new_lines.push(format!("{key}={v}"));
            }
        }
    }

    let out = if new_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", new_lines.join("\n"))
    };
    fs::write(path, out)?;
    Ok(backup)
}

fn draw_ui(frame: &mut ratatui::Frame<'_>, app: &SetupApp) {
    if app.completed {
        let done = Paragraph::new(vec![
            Line::from(Span::styled(
                "✅ Setup saved successfully",
                Style::default()
                    .fg(Color::LightGreen)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("Checks:"),
            Line::from(
                app.completion_summary
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "Config validated".into()),
            ),
            Line::from(app.completion_summary.get(1).cloned().unwrap_or_default()),
            Line::from(""),
            Line::from(format!(
                "Backup: {}",
                app.backup_path.as_deref().unwrap_or("none")
            )),
            Line::from(""),
            Line::from("Next:"),
            Line::from("  1) microclaw start"),
            Line::from(""),
            Line::from("Press Enter to finish."),
        ])
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Setup Complete"),
        );
        frame.render_widget(done, frame.area().inner(Margin::new(2, 2)));
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Min(14),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let header = Paragraph::new(vec![
        Line::from(Span::styled(
            "MicroClaw • Interactive Setup",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(vec![
            Span::styled(
                format!(
                    "Field {}/{}  ·  Section: {}  ·  ",
                    app.selected + 1,
                    app.fields.len(),
                    app.current_section()
                ),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(app.progress_bar(16), Style::default().fg(Color::LightCyan)),
        ]),
    ])
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(header, chunks[0]);

    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(chunks[1]);

    let mut lines = Vec::<Line>::new();
    for (i, f) in app.fields.iter().enumerate() {
        let selected = i == app.selected;
        let label = if f.required {
            format!("{}  [required]", f.label)
        } else {
            f.label.to_string()
        };
        let value = f.display_value(selected && app.editing);
        let prefix = if selected { "▶" } else { " " };
        let color = if selected {
            Color::Yellow
        } else {
            Color::White
        };
        lines.push(Line::from(vec![
            Span::styled(format!("{prefix} {label}: "), Style::default().fg(color)),
            Span::styled(value, Style::default().fg(Color::Green)),
        ]));
    }
    let body = Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title("Fields"))
        .wrap(Wrap { trim: false });
    frame.render_widget(body, body_chunks[0].inner(Margin::new(1, 0)));

    let field = app.selected_field();
    let help = Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Key: ", Style::default().fg(Color::DarkGray)),
            Span::styled(field.key, Style::default().fg(Color::Magenta)),
        ]),
        Line::from(vec![
            Span::styled("Required: ", Style::default().fg(Color::DarkGray)),
            Span::raw(if field.required { "yes" } else { "no" }),
        ]),
        Line::from(vec![
            Span::styled("Editing: ", Style::default().fg(Color::DarkGray)),
            Span::raw(if app.editing { "active" } else { "idle" }),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "Tips",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from("• Enter: edit/save current field"),
        Line::from("• Tab / Shift+Tab: next/previous field"),
        Line::from("• ←/→ on provider: anthropic/openai"),
        Line::from("• F2: validate + online checks"),
        Line::from("• s or Ctrl+S: save to .env"),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title("Details / Help"),
    )
    .wrap(Wrap { trim: false });
    frame.render_widget(help, body_chunks[1].inner(Margin::new(1, 0)));

    let (status_icon, status_color) =
        if app.status.contains("failed") || app.status.contains("Cannot save") {
            ("✖ ", Color::LightRed)
        } else if app.status.contains("saved") || app.status.contains("Saved") {
            ("✔ ", Color::LightGreen)
        } else {
            ("• ", Color::White)
        };
    let status = Paragraph::new(vec![Line::from(vec![
        Span::styled(status_icon, Style::default().fg(status_color)),
        Span::styled(app.status.clone(), Style::default().fg(status_color)),
    ])])
    .block(Block::default().borders(Borders::ALL).title("Status"));
    frame.render_widget(status, chunks[2]);
}

fn try_save(app: &mut SetupApp) {
    match app
        .validate_local()
        .and_then(|_| app.validate_online())
        .and_then(|checks| {
            let values = app.to_env_map();
            let backup = save_env_file(Path::new(".env"), &values)?;
            app.backup_path = backup;
            app.completion_summary = checks;
            Ok(())
        }) {
        Ok(_) => {
            app.status = "Saved .env".into();
            app.completed = true;
        }
        Err(e) => app.status = format!("Cannot save: {e}"),
    }
}

fn run_wizard(mut terminal: DefaultTerminal) -> Result<bool, MicroClawError> {
    let mut app = SetupApp::new();

    loop {
        terminal.draw(|f| draw_ui(f, &app))?;
        if event::poll(Duration::from_millis(250))? {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }

            if app.completed {
                match key.code {
                    KeyCode::Enter | KeyCode::Char('q') => return Ok(true),
                    _ => continue,
                }
            }

            if app.editing {
                match key.code {
                    KeyCode::Esc => {
                        app.editing = false;
                        app.status = "Edit canceled".into();
                    }
                    KeyCode::Enter => {
                        app.editing = false;
                        app.status = format!("Updated {}", app.selected_field().key);
                    }
                    KeyCode::Backspace => {
                        app.selected_field_mut().value.pop();
                    }
                    KeyCode::Char(c) => {
                        app.selected_field_mut().value.push(c);
                    }
                    _ => {}
                }
                continue;
            }

            match key.code {
                KeyCode::Char('q') => return Ok(false),
                KeyCode::Up => app.prev(),
                KeyCode::Down => app.next(),
                KeyCode::Tab => app.next(),
                KeyCode::BackTab => app.prev(),
                KeyCode::Enter => {
                    app.editing = true;
                    app.status = format!("Editing {}", app.selected_field().key);
                }
                KeyCode::Left => {
                    if app.selected_field().key == "LLM_PROVIDER" {
                        app.set_provider("anthropic");
                        app.status = "Provider set to anthropic".into();
                    }
                }
                KeyCode::Right => {
                    if app.selected_field().key == "LLM_PROVIDER" {
                        app.set_provider("openai");
                        app.status = "Provider set to openai".into();
                    }
                }
                KeyCode::F(2) => match app.validate_local().and_then(|_| app.validate_online()) {
                    Ok(checks) => app.status = format!("Validation passed: {}", checks.join(" | ")),
                    Err(e) => app.status = format!("Validation failed: {e}"),
                },
                KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    try_save(&mut app);
                }
                KeyCode::Char('s') => {
                    try_save(&mut app);
                }
                _ => {}
            }
        }
    }
}

pub fn run_setup_wizard() -> Result<bool, MicroClawError> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let terminal = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(stdout))?;
    let result = run_wizard(terminal);
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mask_secret() {
        assert_eq!(mask_secret("abcdefghi"), "abc***hi");
        assert_eq!(mask_secret("abc"), "***");
    }

    #[test]
    fn test_save_env_file() {
        let env_path = std::env::temp_dir().join(format!(
            "microclaw_setup_test_{}.env",
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        fs::write(
            &env_path,
            "FOO=bar\nTELEGRAM_BOT_TOKEN=old\n# comment\nBOT_USERNAME=old_bot\n",
        )
        .unwrap();

        let mut values = HashMap::new();
        values.insert("TELEGRAM_BOT_TOKEN".into(), "new_tok".into());
        values.insert("BOT_USERNAME".into(), "new_bot".into());
        values.insert("LLM_PROVIDER".into(), "anthropic".into());
        values.insert("LLM_API_KEY".into(), "key".into());

        let backup = save_env_file(&env_path, &values).unwrap();
        assert!(backup.is_some());

        let s = fs::read_to_string(&env_path).unwrap();
        assert!(s.contains("FOO=bar"));
        assert!(s.contains("TELEGRAM_BOT_TOKEN=new_tok"));
        assert!(s.contains("BOT_USERNAME=new_bot"));
        assert!(s.contains("LLM_PROVIDER=anthropic"));
        assert!(s.contains("LLM_API_KEY=key"));

        let _ = fs::remove_file(&env_path);
    }
}
