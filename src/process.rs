//! Managed SSH process lifecycle helpers.
//!
//! The GUI owns every SSH child process it starts. This module provides a small
//! thread-based monitor that keeps the Slint event loop unblocked while still
//! reporting stdout, stderr, and exit status back to the application state.

use crate::config::ForwardPreset;
use crate::ssh::SshCommandSpec;
use anyhow::{Context, Result, bail};
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

/// Events emitted by a managed SSH child process.
#[derive(Debug, Clone)]
pub enum ProcessEvent {
    /// The child process was spawned successfully.
    Started {
        /// Preset name associated with the process.
        preset_name: String,
        /// Operating-system process id.
        pid: u32,
    },
    /// A line of stdout or stderr was captured.
    Output {
        /// Preset name associated with the process.
        preset_name: String,
        /// Output stream name, currently `stdout` or `stderr`.
        stream: &'static str,
        /// Captured line.
        line: String,
    },
    /// The child process exited on its own.
    Exited {
        /// Preset name associated with the process.
        preset_name: String,
        /// Human-readable exit status.
        status: String,
        /// Whether the exit status was successful.
        success: bool,
    },
    /// The child process was stopped after a user request.
    Stopped {
        /// Preset name associated with the process.
        preset_name: String,
        /// Human-readable exit status.
        status: String,
    },
}

/// Handle used by the GUI to request a process stop.
#[derive(Debug)]
pub struct ManagedProcess {
    preset_name: String,
    pid: u32,
    stop_tx: Sender<()>,
}

impl ManagedProcess {
    /// Returns the preset name associated with this process.
    pub fn preset_name(&self) -> &str {
        &self.preset_name
    }

    /// Returns the operating-system process id.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Requests process termination.
    ///
    /// The monitor thread performs the actual kill/wait sequence and emits the
    /// final process event.
    pub fn request_stop(&self) {
        let _ = self.stop_tx.send(());
    }
}

impl Drop for ManagedProcess {
    fn drop(&mut self) {
        let _ = self.stop_tx.send(());
    }
}

/// Spawns an SSH process and starts monitor threads for output and lifecycle.
pub fn spawn_managed(
    preset: &ForwardPreset,
    spec: &SshCommandSpec,
    event_tx: Sender<ProcessEvent>,
) -> Result<ManagedProcess> {
    if spec.program.trim().is_empty() {
        bail!("SSH binary path is empty");
    }

    let mut command = Command::new(&spec.program);
    command
        .args(&spec.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .with_context(|| format!("Failed to start SSH command: {}", spec.summary()))?;

    let pid = child.id();
    let preset_name = preset.name.clone();
    let (stop_tx, stop_rx) = mpsc::channel();

    if let Some(stdout) = child.stdout.take() {
        spawn_output_reader(preset_name.clone(), "stdout", stdout, event_tx.clone());
    }
    if let Some(stderr) = child.stderr.take() {
        spawn_output_reader(preset_name.clone(), "stderr", stderr, event_tx.clone());
    }

    let monitor_name = preset_name.clone();
    let monitor_tx = event_tx.clone();
    thread::spawn(move || {
        let _ = monitor_tx.send(ProcessEvent::Started {
            preset_name: monitor_name.clone(),
            pid,
        });

        loop {
            if stop_rx.try_recv().is_ok() {
                let _ = child.kill();
                let status = child.wait();
                let _ = monitor_tx.send(ProcessEvent::Stopped {
                    preset_name: monitor_name,
                    status: format_wait_result(status),
                });
                break;
            }

            match child.try_wait() {
                Ok(Some(status)) => {
                    let _ = monitor_tx.send(ProcessEvent::Exited {
                        preset_name: monitor_name,
                        status: format_exit_status(status),
                        success: status.success(),
                    });
                    break;
                }
                Ok(None) => thread::sleep(Duration::from_millis(200)),
                Err(error) => {
                    let _ = monitor_tx.send(ProcessEvent::Exited {
                        preset_name: monitor_name,
                        status: format!("failed to wait for process: {error}"),
                        success: false,
                    });
                    break;
                }
            }
        }
    });

    Ok(ManagedProcess {
        preset_name,
        pid,
        stop_tx,
    })
}

fn spawn_output_reader<R: Read + Send + 'static>(
    preset_name: String,
    stream: &'static str,
    reader: R,
    event_tx: Sender<ProcessEvent>,
) {
    thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            match line {
                Ok(line) => {
                    let _ = event_tx.send(ProcessEvent::Output {
                        preset_name: preset_name.clone(),
                        stream,
                        line,
                    });
                }
                Err(error) => {
                    let _ = event_tx.send(ProcessEvent::Output {
                        preset_name: preset_name.clone(),
                        stream,
                        line: format!("failed to read {stream}: {error}"),
                    });
                    break;
                }
            }
        }
    });
}

fn format_wait_result(result: std::io::Result<ExitStatus>) -> String {
    match result {
        Ok(status) => format_exit_status(status),
        Err(error) => format!("failed to wait for process: {error}"),
    }
}

fn format_exit_status(status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit code {code}"),
        None => status.to_string(),
    }
}
