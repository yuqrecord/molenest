use crate::config::{Config, ForwardPreset};
use crate::paths;
use crate::session::{self, Session, SessionStatus};
use crate::ssh;
use crate::ui;
use anyhow::{Context, Result, anyhow};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Parser)]
#[command(name = "molenest")]
#[command(about = "Start SSH port forwarding from reusable presets")]
pub struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Start {
        name: Option<String>,
    },
    Stop {
        session_or_name: String,
    },
    List,
    Sessions,
    Add,
    Remove {
        name: String,
    },
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    Doctor,
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    Path,
    Edit,
}

impl Cli {
    pub fn run(self) -> Result<()> {
        match self.command {
            None => start(None),
            Some(Commands::Start { name }) => start(name),
            Some(Commands::Stop { session_or_name }) => stop(&session_or_name),
            Some(Commands::List) => list_presets(),
            Some(Commands::Sessions) => list_sessions(),
            Some(Commands::Add) => add_preset(),
            Some(Commands::Remove { name }) => remove_preset(&name),
            Some(Commands::Config { command }) => match command {
                ConfigCommand::Path => config_path(),
                ConfigCommand::Edit => config_edit(),
            },
            Some(Commands::Doctor) => doctor(),
        }
    }
}

fn start(name: Option<String>) -> Result<()> {
    let config_path = paths::config_file()?;
    let Some(config) = load_config_or_prompt_create(&config_path)? else {
        return Ok(());
    };
    if config.forwards.is_empty() {
        print_no_presets_message();
        return Ok(());
    }
    let preset = match name {
        Some(name) => find_preset(&config, &name)?.clone(),
        None => ui::select_preset(&config)?,
    };
    start_preset(&config, &preset)
}

fn stop(selector: &str) -> Result<()> {
    let sessions_dir = paths::sessions_dir()?;
    let stopped = session::stop_session(&sessions_dir, selector)?;
    println!("Stopped session {}.", stopped.id);
    Ok(())
}

fn list_presets() -> Result<()> {
    let config_path = paths::config_file()?;
    let Some(config) = load_config_or_prompt_create(&config_path)? else {
        return Ok(());
    };
    if config.forwards.is_empty() {
        print_no_presets_message();
        return Ok(());
    }

    println!(
        "{:<20} {:<18} {:<12} {:<24}",
        "NAME", "HOST", "LOCAL PORT", "REMOTE"
    );
    for preset in config.forwards {
        println!(
            "{:<20} {:<18} {:<12} {}:{}",
            preset.name, preset.host, preset.local_port, preset.remote_host, preset.remote_port
        );
    }
    Ok(())
}

fn list_sessions() -> Result<()> {
    let sessions_dir = paths::sessions_dir()?;
    let records = session::list_sessions(&sessions_dir)?;
    if records.is_empty() {
        println!("No known sessions.");
        return Ok(());
    }

    println!(
        "{:<12} {:<20} {:<12} {:<18} {:<10}",
        "SESSION", "PRESET", "LOCAL PORT", "HOST", "STATUS"
    );
    for record in &records {
        let status = match record.status {
            SessionStatus::Running => "running",
            SessionStatus::Stale => "stale",
        };
        println!(
            "{:<12} {:<20} {:<12} {:<18} {}",
            short_id(&record.session.id.to_string()),
            record.session.preset_name,
            record.session.local_port,
            record.session.ssh_host,
            status
        );
    }
    session::cleanup_stale_sessions(&records)?;
    Ok(())
}

fn add_preset() -> Result<()> {
    let config_path = paths::config_file()?;
    let Some(mut config) = load_config_or_prompt_create(&config_path)? else {
        return Ok(());
    };
    let preset = ui::prompt_new_preset()?;
    if config.find_preset(&preset.name).is_some() {
        return Err(anyhow!("Preset already exists: {}", preset.name));
    }
    config.forwards.push(preset.clone());
    config.save(&config_path)?;
    println!("Added preset {}.", preset.name);
    Ok(())
}

fn remove_preset(name: &str) -> Result<()> {
    let config_path = paths::config_file()?;
    let Some(mut config) = load_config_or_prompt_create(&config_path)? else {
        return Ok(());
    };
    if config.forwards.is_empty() {
        print_no_presets_message();
        return Ok(());
    }
    let removed = config.remove_preset(name)?;
    config.save(&config_path)?;
    println!("Removed preset {}.", removed.name);
    Ok(())
}

fn config_path() -> Result<()> {
    println!("{}", paths::config_file()?.display());
    Ok(())
}

fn config_edit() -> Result<()> {
    let path = paths::config_file()?;
    if !path.exists() {
        if !create_config_after_confirmation(&path)? {
            print_config_required_message(&path);
            return Ok(());
        }
    } else {
        paths::ensure_parent(&path)?;
    }
    open_editor(&path)
}

fn doctor() -> Result<()> {
    let config_path = paths::config_file()?;
    println!("Config path: {}", config_path.display());

    let Some(config) = load_config_or_prompt_create(&config_path)? else {
        return Ok(());
    };
    match config.validate() {
        Ok(()) => println!("Config: ok"),
        Err(error) => println!("Config: error: {error:#}"),
    }

    match ssh::ensure_ssh_available(&config.defaults.ssh_binary) {
        Ok(()) => println!("SSH executable: ok ({})", config.defaults.ssh_binary),
        Err(error) => println!("SSH executable: error: {error:#}"),
    }

    for preset in &config.forwards {
        match ssh::ensure_local_port_available(preset.bind_address.as_deref(), preset.local_port) {
            Ok(()) => println!("Port {} for {}: available", preset.local_port, preset.name),
            Err(error) => println!("Port {} for {}: {error:#}", preset.local_port, preset.name),
        }
    }

    println!("Sessions path: {}", paths::sessions_dir()?.display());
    Ok(())
}

fn start_preset(config: &Config, preset: &ForwardPreset) -> Result<()> {
    ssh::ensure_ssh_available(&config.defaults.ssh_binary)?;
    ssh::ensure_local_port_available(preset.bind_address.as_deref(), preset.local_port)?;
    let spec = ssh::build_ssh_command(&config.defaults.ssh_binary, preset);
    let child = ssh::spawn_background(&spec)?;
    let session = Session::new(preset, child.id(), spec.summary());
    session::save_session(&session, &paths::sessions_dir()?)?;

    println!("Started {} in the background.", preset.name);
    println!("Local URL: {}", preset.local_url());
    println!("Session: {}", session.id);
    Ok(())
}

fn load_config_or_prompt_create(path: &Path) -> Result<Option<Config>> {
    if path.exists() {
        return Config::load(path).map(Some).with_context(|| {
            format!(
                "Config file is invalid. Edit it with `molenest config edit`: {}",
                path.display()
            )
        });
    }

    if create_config_after_confirmation(path)? {
        Config::load(path).map(Some)
    } else {
        print_config_required_message(path);
        Ok(None)
    }
}

fn create_config_after_confirmation(path: &Path) -> Result<bool> {
    if ui::confirm_create_config(path)? {
        Config::default().save(path)?;
        println!("Created config file: {}", path.display());
        Ok(true)
    } else {
        Ok(false)
    }
}

fn print_config_required_message(path: &Path) {
    println!("Config file is required to run molenest.");
    println!("No config file was created: {}", path.display());
    println!("Run `molenest config edit` or `molenest add` when ready.");
}

fn print_no_presets_message() {
    println!("No forwarding presets are configured.");
    println!("Edit the config with `molenest config edit` or add one with `molenest add`.");
}

fn find_preset<'a>(config: &'a Config, name: &str) -> Result<&'a ForwardPreset> {
    config
        .find_preset(name)
        .ok_or_else(|| unknown_preset_error(config, name))
}

fn unknown_preset_error(config: &Config, name: &str) -> anyhow::Error {
    let suggestions: Vec<_> = config
        .forwards
        .iter()
        .filter(|preset| preset.name.contains(name) || name.contains(&preset.name))
        .map(|preset| preset.name.as_str())
        .collect();

    if suggestions.is_empty() {
        anyhow!("Unknown preset: {}", name)
    } else {
        anyhow!(
            "Unknown preset: {}. Did you mean {}?",
            name,
            suggestions.join(", ")
        )
    }
}

fn open_editor(path: &PathBuf) -> Result<()> {
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| fallback_editor());
    let status = Command::new(&editor)
        .arg(path)
        .status()
        .with_context(|| format!("Failed to open editor `{}`", editor))?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow!("Editor exited with status {}", status))
    }
}

fn fallback_editor() -> String {
    if cfg!(windows) {
        "notepad".to_string()
    } else {
        "vi".to_string()
    }
}

fn short_id(id: &str) -> &str {
    &id[..id.len().min(8)]
}
