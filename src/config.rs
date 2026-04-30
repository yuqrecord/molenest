use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub forwards: Vec<ForwardPreset>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Defaults {
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ForwardPreset {
    pub name: String,
    pub host: String,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind_address: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_args: Vec<String>,
}

fn default_ssh_binary() -> String {
    "ssh".to_string()
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file {}", path.display()))?;
        let config: Self = toml::from_str(&contents)
            .with_context(|| format!("Failed to parse config file {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn load_or_default(path: &Path) -> Result<Self> {
        if path.exists() {
            Self::load(path)
        } else {
            Ok(Self::default())
        }
    }

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

    pub fn find_preset(&self, name: &str) -> Option<&ForwardPreset> {
        self.forwards.iter().find(|preset| preset.name == name)
    }

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
    pub fn validate(&self) -> Result<()> {
        validate_name(&self.name)?;
        validate_non_empty("host", &self.host)?;
        validate_non_empty("remote_host", &self.remote_host)?;
        if let Some(bind_address) = &self.bind_address {
            validate_non_empty("bind_address", bind_address)?;
        }
        Ok(())
    }

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
