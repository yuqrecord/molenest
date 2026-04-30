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
use std::process::Command;
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
    let edit_state = Rc::clone(&state);
    window.on_edit_config_requested(move || {
        let result = edit_state.borrow_mut().edit_config();
        handle_callback_result(result, &edit_state);
        if let Some(window) = window_weak.upgrade() {
            sync_window(&window, &edit_state.borrow());
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
        let command_summary = spec.summary();
        let handle = process::spawn_managed(&preset, &spec, event_tx)?;
        let pid = handle.pid();

        let connection = self.connections.entry(preset.name.clone()).or_default();
        connection.status = ConnectionStatus::Starting;
        connection.pid = Some(pid);
        connection.started_at = Some(OffsetDateTime::now_utc());
        connection.command_summary = command_summary;
        connection.output.clear();
        connection.push_output("molenest", format!("started ssh process {pid}"));

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
        connection.push_output("molenest", "stop requested".to_string());
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

    fn edit_config(&mut self) -> Result<()> {
        if !self.config_path.exists() {
            Config::default().save(&self.config_path)?;
        }
        open_config_editor(&self.config_path)?;
        self.status_message = format!("Opened config: {}", self.config_path.display());
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
                connection.push_output("molenest", format!("process {pid} is running"));
                self.status_message = format!("{preset_name} is running.");
            }
            ProcessEvent::Output {
                preset_name,
                stream,
                line,
            } => {
                self.connections
                    .entry(preset_name)
                    .or_default()
                    .push_output(stream, line);
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
                connection.push_output("ssh", format!("exited with {status}"));
                self.status_message = format!("{preset_name} exited with {status}.");
            }
            ProcessEvent::Stopped {
                preset_name,
                status,
            } => {
                self.handles.remove(&preset_name);
                let connection = self.connections.entry(preset_name.clone()).or_default();
                connection.status = ConnectionStatus::Stopped;
                connection.push_output("molenest", format!("stopped with {status}"));
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
    command_summary: String,
    output: Vec<String>,
}

impl ConnectionState {
    fn push_output(&mut self, stream: &str, line: String) {
        self.output.push(format!("[{stream}] {line}"));
        const MAX_OUTPUT_LINES: usize = 120;
        if self.output.len() > MAX_OUTPUT_LINES {
            let overflow = self.output.len() - MAX_OUTPUT_LINES;
            self.output.drain(0..overflow);
        }
    }
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
            command: SharedString::from(""),
            started_at: SharedString::from(""),
            output: SharedString::from(""),
        };
    };

    let connection = state.connection_for(&preset.name);
    let command = if connection.command_summary.is_empty() {
        ssh::build_ssh_command(&state.config.defaults.ssh_binary, preset).summary()
    } else {
        connection.command_summary.clone()
    };

    let started_at = connection
        .started_at
        .and_then(|value| value.format(&Rfc3339).ok())
        .unwrap_or_default();

    ConnectionDetails {
        name: preset.name.as_str().into(),
        status: status_with_pid(connection.status, connection.pid).into(),
        local_url: preset.local_url().into(),
        command: command.into(),
        started_at: started_at.into(),
        output: connection.output.join("\n").into(),
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

fn open_config_editor(path: &Path) -> Result<()> {
    paths::ensure_parent(path)?;
    let (program, args) = editor_command(path);
    Command::new(&program)
        .args(args)
        .spawn()
        .with_context(|| format!("Failed to open config editor `{program}`"))?;
    Ok(())
}

fn editor_command(path: &Path) -> (String, Vec<String>) {
    if let Ok(editor) = std::env::var("EDITOR").or_else(|_| std::env::var("VISUAL"))
        && !editor.trim().is_empty()
        && !editor.contains(char::is_whitespace)
    {
        return (editor, vec![path.display().to_string()]);
    }

    if cfg!(windows) {
        ("notepad".to_string(), vec![path.display().to_string()])
    } else if cfg!(target_os = "macos") {
        (
            "open".to_string(),
            vec!["-t".to_string(), path.display().to_string()],
        )
    } else {
        ("xdg-open".to_string(), vec![path.display().to_string()])
    }
}
