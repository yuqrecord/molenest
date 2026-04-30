use anyhow::{Context, Result, bail};
use std::os::windows::process::CommandExt;
use std::process::Command;

const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
const DETACHED_PROCESS: u32 = 0x0000_0008;

pub fn prepare_background_command(command: &mut Command) {
    command.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS);
}

pub fn is_process_running(pid: u32) -> bool {
    Command::new("tasklist")
        .args(["/FI", &format!("PID eq {}", pid), "/NH"])
        .output()
        .map(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout)
                    .to_ascii_lowercase()
                    .contains(&pid.to_string())
        })
        .unwrap_or(false)
}

pub fn terminate_process(pid: u32) -> Result<()> {
    let status = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .status()
        .with_context(|| format!("Failed to run taskkill for pid {}", pid))?;
    if status.success() {
        Ok(())
    } else {
        bail!("Failed to stop process {}", pid)
    }
}
