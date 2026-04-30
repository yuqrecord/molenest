use anyhow::{Context, Result, bail};
use directories::UserDirs;
use std::env;
use std::path::PathBuf;

/// Returns the platform-appropriate `molenest` config file path.
pub fn config_file() -> Result<PathBuf> {
    Ok(config_dir()?.join("molenest").join("config.toml"))
}

fn config_dir() -> Result<PathBuf> {
    if let Some(value) = non_empty_env("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(value));
    }
    Ok(home_dir()?.join(".config"))
}

fn home_dir() -> Result<PathBuf> {
    if let Some(user_dirs) = UserDirs::new() {
        return Ok(user_dirs.home_dir().to_path_buf());
    }
    if let Some(value) = non_empty_env("USERPROFILE") {
        return Ok(PathBuf::from(value));
    }
    if let Some(value) = non_empty_env("HOME") {
        return Ok(PathBuf::from(value));
    }
    bail!("Could not determine home directory")
}

fn non_empty_env(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

/// Creates the parent directory for `path` when it exists.
pub fn ensure_parent(path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create directory {}", parent.display()))?;
    }
    Ok(())
}
