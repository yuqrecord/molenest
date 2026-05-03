//! Slint desktop application wiring.
//!
//! This module owns the UI state, maps configuration presets into Slint models,
//! and coordinates managed SSH child processes.

use crate::config::{Config, ForwardPreset};
use crate::paths;
use crate::process::{self, ManagedProcess, ProcessEvent};
use crate::ssh;
use anyhow::{Context, Result, anyhow};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;
use time::OffsetDateTime;
use time::macros::format_description;

slint::include_modules!();

/// Runs the `molenest` desktop application.
pub fn run() -> Result<()> {
    let window = MainWindow::new().context("Failed to create main window")?;
    let (event_tx, event_rx) = mpsc::channel();
    let state = Rc::new(RefCell::new(AppState::load()?));

    sync_window(&window, &state.borrow());
    connect_callbacks(&window, Rc::clone(&state), event_tx);
    let _event_timer = start_event_timer(&window, Rc::clone(&state), event_rx);

    let run_result = window.run().context("Failed to run Slint event loop");
    state.borrow_mut().stop_all();
    run_result
}

fn connect_callbacks(
    window: &MainWindow,
    state: Rc<RefCell<AppState>>,
    event_tx: Sender<ProcessEvent>,
) {
    let window_weak = window.as_weak();
    let toggle_state = Rc::clone(&state);
    let toggle_tx = event_tx.clone();
    window.on_toggle_requested(move |index| {
        let result = toggle_state
            .borrow_mut()
            .toggle_preset(index, toggle_tx.clone());
        handle_main_callback_result(result, &toggle_state);
        if let Some(window) = window_weak.upgrade() {
            sync_window(&window, &toggle_state.borrow());
        }
    });

    let window_weak = window.as_weak();
    let reload_state = Rc::clone(&state);
    window.on_reload_requested(move || {
        let result = reload_state.borrow_mut().reload_config();
        handle_main_callback_result(result, &reload_state);
        if let Some(window) = window_weak.upgrade() {
            sync_window(&window, &reload_state.borrow());
        }
    });

    let window_weak = window.as_weak();
    let open_add_state = Rc::clone(&state);
    window.on_open_add_preset_requested(move || {
        open_add_state.borrow_mut().add_preset_message.clear();
        if let Some(window) = window_weak.upgrade() {
            window.set_add_dialog_open(true);
            sync_window(&window, &open_add_state.borrow());
        }
    });

    let window_weak = window.as_weak();
    let add_state = Rc::clone(&state);
    window.on_add_preset_requested(move || {
        if let Some(window) = window_weak.upgrade() {
            let draft = PresetDraft {
                name: window.get_draft_name().to_string(),
                host: window.get_draft_host().to_string(),
                local_port: window.get_draft_local_port().to_string(),
                remote_host: window.get_draft_remote_host().to_string(),
                remote_port: window.get_draft_remote_port().to_string(),
            };
            let result = add_state.borrow_mut().add_preset(draft);
            let added = result.is_ok();
            handle_add_preset_result(result, &add_state);
            if added {
                clear_draft_fields(&window);
                window.set_add_dialog_open(false);
            }
            sync_window(&window, &add_state.borrow());
        }
    });

    let window_weak = window.as_weak();
    let edit_open_state = Rc::clone(&state);
    window.on_edit_preset_open_requested(move |index| {
        let result = edit_open_state.borrow_mut().open_edit_preset(index);
        match result {
            Ok(preset) => {
                if let Some(window) = window_weak.upgrade() {
                    populate_edit_fields(&window, &preset);
                    window.set_delete_confirm_open(false);
                    window.set_edit_dialog_open(true);
                    sync_window(&window, &edit_open_state.borrow());
                }
            }
            Err(error) => {
                edit_open_state.borrow_mut().status_message = format!("{error:#}");
                if let Some(window) = window_weak.upgrade() {
                    sync_window(&window, &edit_open_state.borrow());
                }
            }
        }
    });

    let window_weak = window.as_weak();
    let edit_save_state = Rc::clone(&state);
    window.on_edit_preset_save_requested(move || {
        if let Some(window) = window_weak.upgrade() {
            let draft = PresetDraft {
                name: window.get_edit_name().to_string(),
                host: window.get_edit_host().to_string(),
                local_port: window.get_edit_local_port().to_string(),
                remote_host: window.get_edit_remote_host().to_string(),
                remote_port: window.get_edit_remote_port().to_string(),
            };
            let result = edit_save_state.borrow_mut().update_edit_preset(draft);
            let saved = result.is_ok();
            handle_edit_preset_result(result, &edit_save_state);
            if saved {
                clear_edit_fields(&window);
                window.set_delete_confirm_open(false);
                window.set_edit_dialog_open(false);
            }
            sync_window(&window, &edit_save_state.borrow());
        }
    });

    let window_weak = window.as_weak();
    let edit_delete_state = Rc::clone(&state);
    window.on_edit_preset_delete_requested(move || {
        if let Some(window) = window_weak.upgrade() {
            let result = edit_delete_state.borrow_mut().delete_edit_preset();
            let deleted = result.is_ok();
            handle_edit_preset_result(result, &edit_delete_state);
            if deleted {
                clear_edit_fields(&window);
                window.set_delete_confirm_open(false);
                window.set_edit_dialog_open(false);
            } else {
                window.set_delete_confirm_open(false);
            }
            sync_window(&window, &edit_delete_state.borrow());
        }
    });
}

fn start_event_timer(
    window: &MainWindow,
    state: Rc<RefCell<AppState>>,
    event_rx: Receiver<ProcessEvent>,
) -> Timer {
    let timer = Timer::default();
    let window_weak = window.as_weak();
    timer.start(TimerMode::Repeated, Duration::from_millis(150), move || {
        let mut changed = false;
        while let Ok(event) = event_rx.try_recv() {
            state.borrow_mut().handle_process_event(event);
            changed = true;
        }

        if changed && let Some(window) = window_weak.upgrade() {
            sync_window(&window, &state.borrow());
        }
    });
    timer
}

fn handle_main_callback_result(result: Result<()>, state: &Rc<RefCell<AppState>>) {
    if let Err(error) = result {
        state.borrow_mut().status_message = format!("{error:#}");
    }
}

fn handle_add_preset_result(result: Result<()>, state: &Rc<RefCell<AppState>>) {
    if let Err(error) = result {
        state.borrow_mut().add_preset_message = format!("{error:#}");
    }
}

fn handle_edit_preset_result(result: Result<()>, state: &Rc<RefCell<AppState>>) {
    if let Err(error) = result {
        state.borrow_mut().edit_preset_message = format!("{error:#}");
    }
}

#[derive(Debug)]
struct AppState {
    config_path: PathBuf,
    config: Config,
    status_message: String,
    add_preset_message: String,
    edit_preset_message: String,
    editing_preset_name: Option<String>,
    connections: HashMap<String, ConnectionState>,
    handles: HashMap<String, ManagedProcess>,
}

impl AppState {
    fn load() -> Result<Self> {
        let config_path = paths::config_file()?;
        let (config, status_message) = load_or_create_config(&config_path)?;
        Ok(Self {
            config_path,
            config,
            status_message,
            add_preset_message: String::new(),
            edit_preset_message: String::new(),
            editing_preset_name: None,
            connections: HashMap::new(),
            handles: HashMap::new(),
        })
    }

    fn start_preset(&mut self, index: i32, event_tx: Sender<ProcessEvent>) -> Result<()> {
        let preset = self.preset_at(index)?.clone();

        if self.handles.contains_key(&preset.name) {
            return Err(anyhow!("{} is already running.", preset.name));
        }

        if let Err(error) = self.preflight_start(&preset) {
            self.mark_failed(&preset.name);
            return Err(error);
        }

        let spec = ssh::build_ssh_command(&self.config.defaults.ssh_binary, &preset);
        let handle = match process::spawn_managed(&preset, &spec, event_tx) {
            Ok(handle) => handle,
            Err(error) => {
                self.mark_failed(&preset.name);
                return Err(error);
            }
        };
        let pid = handle.pid();

        let connection = self.connections.entry(preset.name.clone()).or_default();
        connection.status = ConnectionStatus::Running;
        connection.pid = Some(pid);
        connection.status_changed_at = Some(now());

        self.handles.insert(preset.name.clone(), handle);
        self.status_message = format!("{} is running.", preset.name);
        Ok(())
    }

    fn toggle_preset(&mut self, index: i32, event_tx: Sender<ProcessEvent>) -> Result<()> {
        let preset_name = self.preset_at(index)?.name.clone();
        if self.handles.contains_key(&preset_name) {
            self.stop_preset(index)
        } else {
            self.start_preset(index, event_tx)
        }
    }

    fn stop_preset(&mut self, index: i32) -> Result<()> {
        let preset_name = self.preset_at(index)?.name.clone();

        let handle = self
            .handles
            .get(&preset_name)
            .ok_or_else(|| anyhow!("{preset_name} is not running."))?;
        handle.request_stop();

        self.status_message = format!("Stopping {preset_name}.");
        Ok(())
    }

    fn reload_config(&mut self) -> Result<()> {
        let (config, _) = load_or_create_config(&self.config_path)?;
        self.config = config;
        self.status_message = "Reloaded config.".to_string();
        Ok(())
    }

    fn add_preset(&mut self, draft: PresetDraft) -> Result<()> {
        let preset = draft.into_preset()?;
        if self.config.find_preset(&preset.name).is_some() {
            return Err(anyhow!("Preset already exists: {}", preset.name));
        }

        let mut config = self.config.clone();
        config.forwards.push(preset.clone());
        config.save(&self.config_path)?;
        self.config = config;
        self.status_message = format!("Added preset {}.", preset.name);
        self.add_preset_message.clear();
        Ok(())
    }

    fn open_edit_preset(&mut self, index: i32) -> Result<ForwardPreset> {
        let preset = self.preset_at(index)?.clone();
        self.editing_preset_name = Some(preset.name.clone());
        self.edit_preset_message.clear();
        Ok(preset)
    }

    fn update_edit_preset(&mut self, draft: PresetDraft) -> Result<()> {
        let original_name = self.editing_preset_name()?;
        self.ensure_preset_not_running(&original_name, "editing")?;

        let mut config = self.config.clone();
        let original_index = config
            .forwards
            .iter()
            .position(|preset| preset.name == original_name)
            .ok_or_else(|| anyhow!("Unknown preset: {original_name}"))?;

        let mut preset = draft.into_preset()?;
        if preset.name != original_name && self.config.find_preset(&preset.name).is_some() {
            return Err(anyhow!("Preset already exists: {}", preset.name));
        }

        let original = config.forwards[original_index].clone();
        preset.bind_address = original.bind_address;
        preset.extra_args = original.extra_args;

        config.forwards[original_index] = preset.clone();
        config.save(&self.config_path)?;
        self.config = config;
        if preset.name != original_name {
            self.connections.remove(&original_name);
        }
        self.editing_preset_name = Some(preset.name.clone());
        self.edit_preset_message.clear();
        self.status_message = format!("Updated preset {}.", preset.name);
        Ok(())
    }

    fn delete_edit_preset(&mut self) -> Result<()> {
        let preset_name = self.editing_preset_name()?;
        self.ensure_preset_not_running(&preset_name, "deleting")?;

        let mut config = self.config.clone();
        config.remove_preset(&preset_name)?;
        config.save(&self.config_path)?;
        self.config = config;
        self.connections.remove(&preset_name);
        self.editing_preset_name = None;
        self.edit_preset_message.clear();
        self.status_message = format!("Deleted preset {preset_name}.");
        Ok(())
    }

    fn editing_preset_name(&self) -> Result<String> {
        self.editing_preset_name
            .clone()
            .ok_or_else(|| anyhow!("No preset is selected for editing."))
    }

    fn ensure_preset_not_running(&self, preset_name: &str, action: &str) -> Result<()> {
        if self.handles.contains_key(preset_name) {
            Err(anyhow!("Stop {preset_name} before {action} it."))
        } else {
            Ok(())
        }
    }

    fn preflight_start(&self, preset: &ForwardPreset) -> Result<()> {
        self.config.validate()?;
        ssh::ensure_ssh_available(&self.config.defaults.ssh_binary)?;
        ssh::ensure_local_port_available(preset.bind_address.as_deref(), preset.local_port)?;
        Ok(())
    }

    fn mark_failed(&mut self, preset_name: &str) {
        let connection = self.connections.entry(preset_name.to_string()).or_default();
        connection.status = ConnectionStatus::Failed;
        connection.pid = None;
        connection.status_changed_at = Some(now());
    }

    fn stop_all(&mut self) {
        for handle in self.handles.values() {
            handle.request_stop();
        }
        self.handles.clear();
    }

    fn handle_process_event(&mut self, event: ProcessEvent) {
        match event {
            ProcessEvent::Started { preset_name, pid } => {
                let connection = self.connections.entry(preset_name.clone()).or_default();
                connection.status = ConnectionStatus::Running;
                connection.pid = Some(pid);
                if connection.status_changed_at.is_none() {
                    connection.status_changed_at = Some(now());
                }
                self.status_message = format!("{preset_name} is running.");
            }
            ProcessEvent::Output {
                preset_name,
                stream,
                line,
            } => {
                if stream == "stderr" && !line.trim().is_empty() {
                    self.status_message = format!("{preset_name}: {line}");
                }
            }
            ProcessEvent::Exited {
                preset_name,
                status,
                success: _,
            } => {
                self.handles.remove(&preset_name);
                let connection = self.connections.entry(preset_name.clone()).or_default();
                connection.status = ConnectionStatus::Failed;
                connection.pid = None;
                connection.status_changed_at = Some(now());
                self.status_message = format!("{preset_name} exited with {status}.");
            }
            ProcessEvent::Stopped { preset_name, .. } => {
                self.handles.remove(&preset_name);
                let connection = self.connections.entry(preset_name.clone()).or_default();
                connection.status = ConnectionStatus::Idle;
                connection.pid = None;
                connection.status_changed_at = None;
                self.status_message = format!("{preset_name} stopped.");
            }
        }
    }

    fn preset_at(&self, index: i32) -> Result<&ForwardPreset> {
        if index < 0 {
            return Err(anyhow!("Unknown preset index: {index}"));
        }
        self.config
            .forwards
            .get(index as usize)
            .ok_or_else(|| anyhow!("Unknown preset index: {index}"))
    }

    fn connection_for(&self, preset_name: &str) -> ConnectionState {
        self.connections
            .get(preset_name)
            .cloned()
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Default)]
struct ConnectionState {
    status: ConnectionStatus,
    pid: Option<u32>,
    status_changed_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ConnectionStatus {
    #[default]
    Idle,
    Running,
    Failed,
}

impl ConnectionStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Running => "Running",
            Self::Failed => "Failed",
        }
    }
}

fn sync_window(window: &MainWindow, state: &AppState) {
    let rows = state
        .config
        .forwards
        .iter()
        .map(|preset| {
            let connection = state.connection_for(&preset.name);
            PresetRow {
                name: preset.name.as_str().into(),
                host: preset.host.as_str().into(),
                local_port: preset.local_port.to_string().into(),
                remote: format!("{}:{}", preset.remote_host, preset.remote_port).into(),
                status: connection.status.as_str().into(),
                status_time: status_time(connection).into(),
            }
        })
        .collect::<Vec<_>>();
    let column_widths = column_widths(&rows);

    window.set_presets(ModelRc::from(Rc::new(VecModel::from(rows))));
    window.set_column_widths(column_widths);
    window.set_config_path(state.config_path.display().to_string().into());
    window.set_status_message(state.status_message.as_str().into());
    window.set_add_preset_message(state.add_preset_message.as_str().into());
    window.set_edit_preset_message(state.edit_preset_message.as_str().into());
}

fn column_widths(rows: &[PresetRow]) -> PresetColumnWidths {
    let mut name = estimated_text_width("Name", 13.0);
    let mut local_port = estimated_text_width("Local", 13.0);
    let mut host = estimated_text_width("Host", 13.0);
    let mut remote = estimated_text_width("Remote", 13.0);
    let mut status = estimated_text_width("Status", 13.0);

    for row in rows {
        name = name.max(estimated_text_width(row.name.as_str(), 15.0));
        local_port = local_port.max(estimated_text_width(row.local_port.as_str(), 15.0));
        host = host.max(estimated_text_width(row.host.as_str(), 15.0));
        remote = remote.max(estimated_text_width(row.remote.as_str(), 15.0));
        status = status.max(estimated_text_width(row.status.as_str(), 14.0));
        if !row.status_time.is_empty() {
            status = status.max(estimated_text_width(row.status_time.as_str(), 11.0));
        }
    }

    PresetColumnWidths {
        active: 76.0,
        name: padded_column_width(name, 64.0),
        local_port: padded_column_width(local_port, 56.0),
        host: padded_column_width(host, 64.0),
        remote: padded_column_width(remote, 112.0),
        status: padded_column_width(status, 92.0),
    }
}

fn padded_column_width(content_width: f32, min_width: f32) -> f32 {
    content_width.max(min_width) + 18.0
}

fn estimated_text_width(text: &str, font_size: f32) -> f32 {
    text.chars()
        .map(|value| {
            if value.is_ascii() {
                font_size * 0.58
            } else {
                font_size
            }
        })
        .sum()
}

fn status_time(connection: ConnectionState) -> String {
    match connection.status {
        ConnectionStatus::Running | ConnectionStatus::Failed => connection
            .status_changed_at
            .and_then(|value| value.format(DATE_TIME_FORMAT).ok())
            .unwrap_or_default(),
        ConnectionStatus::Idle => String::new(),
    }
}

fn now() -> OffsetDateTime {
    OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc())
}

const DATE_TIME_FORMAT: &[time::format_description::FormatItem<'_>] =
    format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");

fn load_or_create_config(path: &Path) -> Result<(Config, String)> {
    if path.exists() {
        let config = Config::load(path)?;
        return Ok((config, format!("Loaded config: {}", path.display())));
    }

    let config = Config::default();
    config.save(path)?;
    Ok((
        config,
        format!("Created default config: {}", path.display()),
    ))
}

#[derive(Debug)]
struct PresetDraft {
    name: String,
    host: String,
    local_port: String,
    remote_host: String,
    remote_port: String,
}

impl PresetDraft {
    fn into_preset(self) -> Result<ForwardPreset> {
        let preset = ForwardPreset {
            name: self.name.trim().to_string(),
            host: self.host.trim().to_string(),
            local_port: parse_port("local_port", &self.local_port)?,
            remote_host: self.remote_host.trim().to_string(),
            remote_port: parse_port("remote_port", &self.remote_port)?,
            bind_address: None,
            extra_args: vec![],
        };
        preset.validate()?;
        Ok(preset)
    }
}

fn parse_port(field: &str, value: &str) -> Result<u16> {
    value
        .trim()
        .parse::<u16>()
        .with_context(|| format!("{field} must be a port in the range 1..=65535"))
        .and_then(|port| {
            if port == 0 {
                Err(anyhow!("{field} must be a port in the range 1..=65535"))
            } else {
                Ok(port)
            }
        })
}

fn clear_draft_fields(window: &MainWindow) {
    window.set_draft_name(SharedString::from(""));
    window.set_draft_host(SharedString::from(""));
    window.set_draft_local_port(SharedString::from(""));
    window.set_draft_remote_host(SharedString::from("127.0.0.1"));
    window.set_draft_remote_port(SharedString::from(""));
}

fn populate_edit_fields(window: &MainWindow, preset: &ForwardPreset) {
    window.set_edit_name(SharedString::from(preset.name.as_str()));
    window.set_edit_host(SharedString::from(preset.host.as_str()));
    window.set_edit_local_port(SharedString::from(preset.local_port.to_string()));
    window.set_edit_remote_host(SharedString::from(preset.remote_host.as_str()));
    window.set_edit_remote_port(SharedString::from(preset.remote_port.to_string()));
}

fn clear_edit_fields(window: &MainWindow) {
    window.set_edit_name(SharedString::from(""));
    window.set_edit_host(SharedString::from(""));
    window.set_edit_local_port(SharedString::from(""));
    window.set_edit_remote_host(SharedString::from(""));
    window.set_edit_remote_port(SharedString::from(""));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draft_builds_valid_preset() {
        let preset = PresetDraft {
            name: "jupyter".to_string(),
            host: "my-server".to_string(),
            local_port: "8888".to_string(),
            remote_host: "127.0.0.1".to_string(),
            remote_port: "8888".to_string(),
        }
        .into_preset()
        .unwrap();

        assert_eq!(preset.name, "jupyter");
        assert_eq!(preset.host, "my-server");
        assert_eq!(preset.local_port, 8888);
        assert_eq!(preset.remote_port, 8888);
    }

    #[test]
    fn draft_rejects_invalid_port() {
        let result = PresetDraft {
            name: "bad".to_string(),
            host: "server".to_string(),
            local_port: "0".to_string(),
            remote_host: "127.0.0.1".to_string(),
            remote_port: "8888".to_string(),
        }
        .into_preset();

        assert!(result.is_err());
    }

    #[test]
    fn column_widths_follow_widest_values() {
        let short = column_widths(&[preset_row("app", "h", "127.0.0.1:1")]);
        let long = column_widths(&[preset_row(
            "very-long-notebook-preset",
            "production-gpu-server",
            "127.0.0.1:8888",
        )]);

        assert!(long.name > short.name);
        assert!(long.host > short.host);
        assert!(long.remote > short.remote);
        assert_eq!(short.active, long.active);
    }

    #[test]
    fn edit_updates_selected_preset() {
        let mut state = test_state(vec![forward_preset("old"), forward_preset("other")]);
        state.open_edit_preset(0).unwrap();

        state
            .update_edit_preset(PresetDraft {
                name: "new".to_string(),
                host: "new-host".to_string(),
                local_port: "9999".to_string(),
                remote_host: "127.0.0.1".to_string(),
                remote_port: "9998".to_string(),
            })
            .unwrap();

        let preset = state.config.find_preset("new").unwrap();
        assert_eq!(preset.host, "new-host");
        assert_eq!(preset.local_port, 9999);
        assert!(state.config.find_preset("old").is_none());
        assert_eq!(state.status_message, "Updated preset new.");
    }

    #[test]
    fn edit_rejects_duplicate_preset_name() {
        let mut state = test_state(vec![forward_preset("old"), forward_preset("taken")]);
        state.open_edit_preset(0).unwrap();

        let result = state.update_edit_preset(PresetDraft {
            name: "taken".to_string(),
            host: "server".to_string(),
            local_port: "8888".to_string(),
            remote_host: "127.0.0.1".to_string(),
            remote_port: "8888".to_string(),
        });

        assert!(result.is_err());
        assert!(state.config.find_preset("old").is_some());
        assert!(state.config.find_preset("taken").is_some());
    }

    #[test]
    fn delete_removes_selected_preset() {
        let mut state = test_state(vec![forward_preset("old"), forward_preset("kept")]);
        state.open_edit_preset(0).unwrap();

        state.delete_edit_preset().unwrap();

        assert!(state.config.find_preset("old").is_none());
        assert!(state.config.find_preset("kept").is_some());
        assert_eq!(state.editing_preset_name, None);
        assert_eq!(state.status_message, "Deleted preset old.");
    }

    fn preset_row(name: &str, host: &str, remote: &str) -> PresetRow {
        PresetRow {
            name: name.into(),
            host: host.into(),
            local_port: "8888".into(),
            remote: remote.into(),
            status: "Idle".into(),
            status_time: "".into(),
        }
    }

    fn test_state(forwards: Vec<ForwardPreset>) -> AppState {
        AppState {
            config_path: test_config_path(),
            config: Config {
                defaults: Default::default(),
                forwards,
            },
            status_message: String::new(),
            add_preset_message: String::new(),
            edit_preset_message: String::new(),
            editing_preset_name: None,
            connections: HashMap::new(),
            handles: HashMap::new(),
        }
    }

    fn test_config_path() -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "molenest-edit-test-{}-{nanos}.toml",
            std::process::id()
        ))
    }

    fn forward_preset(name: &str) -> ForwardPreset {
        ForwardPreset {
            name: name.to_string(),
            host: "server".to_string(),
            local_port: 8888,
            remote_host: "127.0.0.1".to_string(),
            remote_port: 8888,
            bind_address: None,
            extra_args: vec![],
        }
    }
}
