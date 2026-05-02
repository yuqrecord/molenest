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
    let start_state = Rc::clone(&state);
    let start_tx = event_tx.clone();
    window.on_start_requested(move |index| {
        let result = start_state
            .borrow_mut()
            .start_preset(index, start_tx.clone());
        handle_callback_result(result, &start_state);
        if let Some(window) = window_weak.upgrade() {
            sync_window(&window, &start_state.borrow());
        }
    });

    let window_weak = window.as_weak();
    let stop_state = Rc::clone(&state);
    window.on_stop_requested(move |index| {
        let result = stop_state.borrow_mut().stop_preset(index);
        handle_callback_result(result, &stop_state);
        if let Some(window) = window_weak.upgrade() {
            sync_window(&window, &stop_state.borrow());
        }
    });

    let window_weak = window.as_weak();
    let reload_state = Rc::clone(&state);
    window.on_reload_requested(move || {
        let result = reload_state.borrow_mut().reload_config();
        handle_callback_result(result, &reload_state);
        if let Some(window) = window_weak.upgrade() {
            sync_window(&window, &reload_state.borrow());
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
            handle_callback_result(result, &add_state);
            if added {
                clear_draft_fields(&window);
                window.set_add_dialog_open(false);
            }
            sync_window(&window, &add_state.borrow());
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

fn handle_callback_result(result: Result<()>, state: &Rc<RefCell<AppState>>) {
    if let Err(error) = result {
        state.borrow_mut().status_message = format!("{error:#}");
    }
}

#[derive(Debug)]
struct AppState {
    config_path: PathBuf,
    config: Config,
    status_message: String,
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

        self.config.forwards.push(preset.clone());
        self.config.save(&self.config_path)?;
        self.status_message = format!("Added preset {}.", preset.name);
        Ok(())
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

    window.set_presets(ModelRc::from(Rc::new(VecModel::from(rows))));
    window.set_config_path(state.config_path.display().to_string().into());
    window.set_status_message(state.status_message.as_str().into());
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
}
