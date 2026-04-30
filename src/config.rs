use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Top-level user configuration loaded from `config.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    /// Application-wide defaults.
    #[serde(default)]
    pub defaults: Defaults,
    /// SSH local forwarding presets shown in the GUI.
    #[serde(default)]
    pub forwards: Vec<ForwardPreset>,
}

/// Default settings applied to all forwarding presets.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Defaults {
    /// SSH executable path or command name.
    #[serde(default = "default_ssh_binary")]
    pub ssh_binary: String,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            ssh_binary: default_ssh_binary(),
        }
    }
}

/// A reusable SSH local port-forwarding preset.
///
/// The `host` field is intentionally the raw OpenSSH destination, usually a
/// `Host` alias from `~/.ssh/config`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ForwardPreset {
    /// Display name used to identify the preset in the GUI.
    pub name: String,
    /// SSH destination passed directly to the `ssh` executable.
    pub host: String,
    /// Local TCP port to bind.
    pub local_port: u16,
    /// Remote-side host passed to `ssh -L`.
    pub remote_host: String,
    /// Remote-side TCP port passed to `ssh -L`.
    pub remote_port: u16,
    /// Optional local bind address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind_address: Option<String>,
    /// Advanced user-provided SSH arguments passed as individual args.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_args: Vec<String>,
}

fn default_ssh_binary() -> String {
    "ssh".to_string()
}

impl Config {
    /// Loads and validates a TOML configuration file.
    pub fn load(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file {}", path.display()))?;
        let config: Self = toml::from_str(&contents)
            .with_context(|| format!("Failed to parse config file {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    /// Loads configuration if present, otherwise returns an empty default.
    pub fn load_or_default(path: &Path) -> Result<Self> {
        if path.exists() {
            Self::load(path)
        } else {
            Ok(Self::default())
        }
    }

    /// Validates and saves configuration as pretty TOML.
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create config directory {}", parent.display())
            })?;
        }
        let contents = toml::to_string_pretty(self).context("Failed to serialize config")?;
        fs::write(path, contents)
            .with_context(|| format!("Failed to write config file {}", path.display()))?;
        Ok(())
    }

    /// Validates global defaults and every forwarding preset.
    pub fn validate(&self) -> Result<()> {
        if self.defaults.ssh_binary.trim().is_empty() {
            bail!("defaults.ssh_binary must not be empty");
        }

        let mut names = HashSet::new();
        for preset in &self.forwards {
            preset.validate()?;
            if !names.insert(preset.name.clone()) {
                bail!("Duplicate preset name: {}", preset.name);
            }
        }
        Ok(())
    }

    /// Finds a preset by exact name.
    pub fn find_preset(&self, name: &str) -> Option<&ForwardPreset> {
        self.forwards.iter().find(|preset| preset.name == name)
    }

    /// Removes and returns a preset by exact name.
    pub fn remove_preset(&mut self, name: &str) -> Result<ForwardPreset> {
        let index = self
            .forwards
            .iter()
            .position(|preset| preset.name == name)
            .ok_or_else(|| anyhow!("Unknown preset: {}", name))?;
        Ok(self.forwards.remove(index))
    }
}

impl ForwardPreset {
    /// Validates preset fields that are not already checked by typed
    /// deserialization.
    pub fn validate(&self) -> Result<()> {
        validate_name(&self.name)?;
        validate_non_empty("host", &self.host)?;
        validate_non_empty("remote_host", &self.remote_host)?;
        if let Some(bind_address) = &self.bind_address {
            validate_non_empty("bind_address", bind_address)?;
        }
        Ok(())
    }

    /// Returns the local HTTP URL commonly used for notebook and dashboard
    /// forwarding presets.
    pub fn local_url(&self) -> String {
        let host = self.bind_address.as_deref().unwrap_or("127.0.0.1");
        format!("http://{}:{}", host, self.local_port)
    }
}

fn validate_name(name: &str) -> Result<()> {
    validate_non_empty("name", name)?;
    if name.chars().any(char::is_whitespace) {
        bail!("Preset name must not contain whitespace: {}", name);
    }
    Ok(())
}

fn validate_non_empty(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        bail!("{} must not be empty", field);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config() {
        let input = r#"
[defaults]
ssh_binary = "ssh"

[[forwards]]
name = "jupyter"
host = "my-server"
local_port = 8888
remote_host = "127.0.0.1"
remote_port = 8888
"#;

        let config: Config = toml::from_str(input).unwrap();
        config.validate().unwrap();
        assert_eq!(config.find_preset("jupyter").unwrap().host, "my-server");
    }

    #[test]
    fn serializes_config() {
        let config = Config {
            defaults: Defaults::default(),
            forwards: vec![ForwardPreset {
                name: "marimo".to_string(),
                host: "gpu".to_string(),
                local_port: 2718,
                remote_host: "127.0.0.1".to_string(),
                remote_port: 2718,
                bind_address: None,
                extra_args: vec![],
            }],
        };

        let output = toml::to_string(&config).unwrap();
        assert!(output.contains("marimo"));
        assert!(output.contains("ssh_binary"));
    }

    #[test]
    fn rejects_empty_name() {
        let preset = ForwardPreset {
            name: "".to_string(),
            host: "server".to_string(),
            local_port: 1,
            remote_host: "127.0.0.1".to_string(),
            remote_port: 1,
            bind_address: None,
            extra_args: vec![],
        };

        assert!(preset.validate().is_err());
    }

    #[test]
    fn finds_preset_by_name() {
        let config = Config {
            defaults: Defaults::default(),
            forwards: vec![ForwardPreset {
                name: "notebook".to_string(),
                host: "server".to_string(),
                local_port: 8888,
                remote_host: "127.0.0.1".to_string(),
                remote_port: 8888,
                bind_address: None,
                extra_args: vec![],
            }],
        };

        assert!(config.find_preset("notebook").is_some());
        assert!(config.find_preset("missing").is_none());
    }
}
