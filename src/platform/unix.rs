use anyhow::{Context, Result, bail};
use std::process::Command;

pub fn prepare_background_command(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        command.pre_exec(|| {
            if libc_setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

pub fn is_process_running(pid: u32) -> bool {
    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn terminate_process(pid: u32) -> Result<()> {
    let status = Command::new("kill")
        .arg(pid.to_string())
        .status()
        .with_context(|| format!("Failed to run kill for pid {}", pid))?;
    if status.success() {
        Ok(())
    } else {
        bail!("Failed to stop process {}", pid)
    }
}

unsafe extern "C" {
    fn setsid() -> i32;
}

fn libc_setsid() -> i32 {
    unsafe { setsid() }
}
