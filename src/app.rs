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
use time::format_description::well_known::Rfc3339;

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
    let select_state = Rc::clone(&state);
    window.on_selected(move |index| {
        select_state.borrow_mut().select(index);
        if let Some(window) = window_weak.upgrade() {
            sync_window(&window, &select_state.borrow());
        }
    });

    let window_weak = window.as_weak();
    let start_state = Rc::clone(&state);
    let start_tx = event_tx.clone();
    window.on_start_requested(move || {
        let result = start_state.borrow_mut().start_selected(start_tx.clone());
        handle_callback_result(result, &start_state);
        if let Some(window) = window_weak.upgrade() {
            sync_window(&window, &start_state.borrow());
        }
    });

    let window_weak = window.as_weak();
    let stop_state = Rc::clone(&state);
    window.on_stop_requested(move || {
        let result = stop_state.borrow_mut().stop_selected();
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

    let window_weak = window.as_weak();
    let doctor_state = Rc::clone(&state);
    window.on_doctor_requested(move || {
        let result = doctor_state.borrow_mut().doctor();
        handle_callback_result(result, &doctor_state);
        if let Some(window) = window_weak.upgrade() {
            sync_window(&window, &doctor_state.borrow());
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
    selected_index: i32,
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
            selected_index: -1,
            status_message,
            connections: HashMap::new(),
            handles: HashMap::new(),
        })
    }

    fn select(&mut self, index: i32) {
        self.selected_index = index;
        if let Some(preset) = self.selected_preset() {
            self.status_message = format!("Selected {}.", preset.name);
        }
    }

    fn start_selected(&mut self, event_tx: Sender<ProcessEvent>) -> Result<()> {
        let preset = self
            .selected_preset()
            .cloned()
            .ok_or_else(|| anyhow!("Select a preset first."))?;

        let existing = self
            .connections
            .entry(preset.name.clone())
            .or_default()
            .status;
        if existing.is_active() {
            return Err(anyhow!("{} is already {}.", preset.name, existing.as_str()));
        }

        ssh::ensure_ssh_available(&self.config.defaults.ssh_binary)?;
        ssh::ensure_local_port_available(preset.bind_address.as_deref(), preset.local_port)?;

        let spec = ssh::build_ssh_command(&self.config.defaults.ssh_binary, &preset);
        let handle = process::spawn_managed(&preset, &spec, event_tx)?;
        let pid = handle.pid();

        let connection = self.connections.entry(preset.name.clone()).or_default();
        connection.status = ConnectionStatus::Starting;
        connection.pid = Some(pid);
        connection.started_at = Some(OffsetDateTime::now_utc());

        self.handles.insert(preset.name.clone(), handle);
        self.status_message = format!("Starting {}.", preset.name);
        Ok(())
    }

    fn stop_selected(&mut self) -> Result<()> {
        let preset_name = self
            .selected_preset()
            .map(|preset| preset.name.clone())
            .ok_or_else(|| anyhow!("Select a preset first."))?;

        let handle = self
            .handles
            .get(&preset_name)
            .ok_or_else(|| anyhow!("{preset_name} is not running."))?;
        handle.request_stop();

        let connection = self.connections.entry(preset_name.clone()).or_default();
        connection.status = ConnectionStatus::Stopping;
        self.status_message = format!("Stopping {preset_name}.");
        Ok(())
    }

    fn reload_config(&mut self) -> Result<()> {
        let (config, _) = load_or_create_config(&self.config_path)?;
        self.config = config;
        if self.selected_index as usize >= self.config.forwards.len() {
            self.selected_index = if self.config.forwards.is_empty() {
                -1
            } else {
                0
            };
        }
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
        self.selected_index = (self.config.forwards.len() - 1) as i32;
        self.status_message = format!("Added preset {}.", preset.name);
        Ok(())
    }

    fn doctor(&mut self) -> Result<()> {
        self.config.validate()?;
        ssh::ensure_ssh_available(&self.config.defaults.ssh_binary)?;

        let mut unavailable = Vec::new();
        for preset in &self.config.forwards {
            if let Err(error) =
                ssh::ensure_local_port_available(preset.bind_address.as_deref(), preset.local_port)
            {
                unavailable.push(format!("{}: {error:#}", preset.name));
            }
        }

        self.status_message = if unavailable.is_empty() {
            format!(
                "Doctor passed. SSH executable `{}` is available.",
                self.config.defaults.ssh_binary
            )
        } else {
            format!("Doctor found port issues: {}", unavailable.join("; "))
        };
        Ok(())
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
                success,
            } => {
                self.handles.remove(&preset_name);
                let connection = self.connections.entry(preset_name.clone()).or_default();
                connection.status = if success {
                    ConnectionStatus::Exited
                } else {
                    ConnectionStatus::Failed
                };
                self.status_message = format!("{preset_name} exited with {status}.");
            }
            ProcessEvent::Stopped { preset_name, .. } => {
                self.handles.remove(&preset_name);
                let connection = self.connections.entry(preset_name.clone()).or_default();
                connection.status = ConnectionStatus::Stopped;
                self.status_message = format!("{preset_name} stopped.");
            }
        }
    }

    fn selected_preset(&self) -> Option<&ForwardPreset> {
        if self.selected_index < 0 {
            return None;
        }
        self.config.forwards.get(self.selected_index as usize)
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
    started_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ConnectionStatus {
    #[default]
    Idle,
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed,
    Exited,
}

impl ConnectionStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
            Self::Exited => "exited",
        }
    }

    fn is_active(self) -> bool {
        matches!(self, Self::Starting | Self::Running | Self::Stopping)
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
            }
        })
        .collect::<Vec<_>>();

    window.set_presets(ModelRc::from(Rc::new(VecModel::from(rows))));
    window.set_selected_index(state.selected_index);
    window.set_config_path(state.config_path.display().to_string().into());
    window.set_status_message(state.status_message.as_str().into());
    window.set_details(details_for_state(state));
}

fn details_for_state(state: &AppState) -> ConnectionDetails {
    let Some(preset) = state.selected_preset() else {
        return ConnectionDetails {
            name: SharedString::from(""),
            status: SharedString::from("No preset selected"),
            local_url: SharedString::from(""),
            started_at: SharedString::from(""),
        };
    };

    let connection = state.connection_for(&preset.name);
    let started_at = connection
        .started_at
        .and_then(|value| value.format(&Rfc3339).ok())
        .unwrap_or_default();

    ConnectionDetails {
        name: preset.name.as_str().into(),
        status: status_with_pid(connection.status, connection.pid).into(),
        local_url: preset.local_url().into(),
        started_at: started_at.into(),
    }
}

fn status_with_pid(status: ConnectionStatus, pid: Option<u32>) -> String {
    match pid {
        Some(pid) if status.is_active() => format!("{} (pid {pid})", status.as_str()),
        _ => status.as_str().to_string(),
    }
}

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
