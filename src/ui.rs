use crate::config::{Config, ForwardPreset};
use anyhow::{Result, anyhow};
use inquire::{Confirm, Select, Text};
use is_terminal::IsTerminal;

pub fn select_preset(config: &Config) -> Result<ForwardPreset> {
    if config.forwards.is_empty() {
        return Err(anyhow!(
            "No forwarding presets configured. Run `molenest add` to create one."
        ));
    }

    let options: Vec<PresetOption> = config.forwards.iter().cloned().map(PresetOption).collect();
    let selected = Select::new("Select forwarding preset:", options).prompt()?;
    Ok(selected.0)
}

pub fn prompt_new_preset() -> Result<ForwardPreset> {
    let name = Text::new("Preset name:").prompt()?;
    let host = Text::new("SSH host alias:").prompt()?;
    let local_port = prompt_port("Local port:")?;
    let remote_host = Text::new("Remote host:")
        .with_default("127.0.0.1")
        .prompt()?;
    let remote_port = prompt_port("Remote port:")?;
    let bind_address = Text::new("Bind address (optional):")
        .with_default("")
        .prompt()?;

    let preset = ForwardPreset {
        name,
        host,
        local_port,
        remote_host,
        remote_port,
        bind_address: if bind_address.trim().is_empty() {
            None
        } else {
            Some(bind_address)
        },
        extra_args: vec![],
    };
    preset.validate()?;
    Ok(preset)
}

pub fn confirm_create_config(path: &std::path::Path) -> Result<bool> {
    if !std::io::stdin().is_terminal() {
        return Ok(false);
    }

    Ok(Confirm::new(&format!(
        "Config file does not exist at {}. Create it?",
        path.display()
    ))
    .with_default(false)
    .prompt()?)
}

fn prompt_port(message: &str) -> Result<u16> {
    loop {
        let input = Text::new(message).prompt()?;
        match input.parse::<u16>() {
            Ok(port) if port > 0 => return Ok(port),
            _ => eprintln!("Enter a port in the range 1..=65535."),
        }
    }
}

#[derive(Clone)]
struct PresetOption(ForwardPreset);

impl std::fmt::Display for PresetOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let preset = &self.0;
        write!(
            f,
            "{:<18} {:<16} {}:{} -> localhost:{}",
            preset.name, preset.host, preset.remote_host, preset.remote_port, preset.local_port
        )
    }
}
