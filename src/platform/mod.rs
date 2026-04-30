#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use unix::*;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(any(unix, windows)))]
pub fn prepare_background_command(_command: &mut std::process::Command) {}

#[cfg(not(any(unix, windows)))]
pub fn is_process_running(_pid: u32) -> bool {
    true
}

#[cfg(not(any(unix, windows)))]
pub fn terminate_process(_pid: u32) -> anyhow::Result<()> {
    anyhow::bail!("Stopping sessions is not supported on this platform")
}
