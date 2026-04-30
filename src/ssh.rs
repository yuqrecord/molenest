use crate::config::ForwardPreset;
use anyhow::{Context, Result};
use std::net::TcpListener;

/// A shell-free SSH command description.
///
/// `program` and `args` are stored separately so callers can pass them directly
/// to `std::process::Command` without constructing a shell string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshCommandSpec {
    /// The SSH executable path or command name.
    pub program: String,
    /// Arguments passed to the SSH executable.
    pub args: Vec<String>,
}

impl SshCommandSpec {
    /// Returns a display-only command summary.
    ///
    /// This string is intended for UI display and diagnostics. It must not be
    /// executed through a shell.
    pub fn summary(&self) -> String {
        let mut parts = Vec::with_capacity(self.args.len() + 1);
        parts.push(self.program.clone());
        parts.extend(self.args.clone());
        parts.join(" ")
    }
}

/// Builds the `ssh -N -L ...` command for a forwarding preset.
///
/// The preset's `host` is preserved as the final SSH destination so OpenSSH can
/// resolve `~/.ssh/config` normally.
pub fn build_ssh_command(ssh_binary: &str, preset: &ForwardPreset) -> SshCommandSpec {
    let forward = match preset.bind_address.as_deref() {
        Some(bind_address) => format!(
            "{}:{}:{}:{}",
            bind_address, preset.local_port, preset.remote_host, preset.remote_port
        ),
        None => format!(
            "{}:{}:{}",
            preset.local_port, preset.remote_host, preset.remote_port
        ),
    };

    let mut args = vec!["-N".to_string(), "-L".to_string(), forward];
    args.extend(preset.extra_args.clone());
    args.push(preset.host.clone());

    SshCommandSpec {
        program: ssh_binary.to_string(),
        args,
    }
}

/// Verifies that the configured SSH executable can be found.
pub fn ensure_ssh_available(ssh_binary: &str) -> Result<()> {
    which::which(ssh_binary).with_context(|| {
        format!(
            "SSH executable not found: {}. Install OpenSSH or set defaults.ssh_binary.",
            ssh_binary
        )
    })?;
    Ok(())
}

/// Checks whether the configured local forwarding port appears available.
///
/// This is a best-effort preflight check. The later `ssh` process may still fail
/// if another process binds the port between this check and process startup.
pub fn ensure_local_port_available(bind_address: Option<&str>, port: u16) -> Result<()> {
    let address = bind_address.unwrap_or("127.0.0.1");
    TcpListener::bind((address, port))
        .with_context(|| format!("Local port {} appears unavailable on {}", port, address))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset() -> ForwardPreset {
        ForwardPreset {
            name: "jupyter".to_string(),
            host: "my-server".to_string(),
            local_port: 8888,
            remote_host: "127.0.0.1".to_string(),
            remote_port: 8888,
            bind_address: None,
            extra_args: vec![],
        }
    }

    #[test]
    fn builds_ssh_command_with_host_alias() {
        let spec = build_ssh_command("ssh", &preset());
        assert_eq!(spec.program, "ssh");
        assert_eq!(
            spec.args,
            vec![
                "-N".to_string(),
                "-L".to_string(),
                "8888:127.0.0.1:8888".to_string(),
                "my-server".to_string()
            ]
        );
    }

    #[test]
    fn builds_ssh_command_with_bind_address() {
        let mut preset = preset();
        preset.bind_address = Some("0.0.0.0".to_string());
        let spec = build_ssh_command("ssh", &preset);
        assert_eq!(spec.args[2], "0.0.0.0:8888:127.0.0.1:8888");
    }
}
