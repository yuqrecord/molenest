use crate::config::ForwardPreset;
use crate::platform;
use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use time::OffsetDateTime;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Session {
    pub id: Uuid,
    pub preset_name: String,
    pub pid: u32,
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
    pub ssh_host: String,
    pub started_at: OffsetDateTime,
    pub command_summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStatus {
    Running,
    Stale,
}

#[derive(Debug, Clone)]
pub struct SessionRecord {
    pub session: Session,
    pub status: SessionStatus,
    pub path: PathBuf,
}

impl Session {
    pub fn new(preset: &ForwardPreset, pid: u32, command_summary: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            preset_name: preset.name.clone(),
            pid,
            local_port: preset.local_port,
            remote_host: preset.remote_host.clone(),
            remote_port: preset.remote_port,
            ssh_host: preset.host.clone(),
            started_at: OffsetDateTime::now_utc(),
            command_summary,
        }
    }
}

pub fn save_session(session: &Session, sessions_dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(sessions_dir).with_context(|| {
        format!(
            "Failed to create sessions directory {}",
            sessions_dir.display()
        )
    })?;
    let path = session_path(sessions_dir, session.id);
    let contents = serde_json_pretty(session)?;
    fs::write(&path, contents)
        .with_context(|| format!("Failed to write session file {}", path.display()))?;
    Ok(path)
}

pub fn list_sessions(sessions_dir: &Path) -> Result<Vec<SessionRecord>> {
    if !sessions_dir.exists() {
        return Ok(vec![]);
    }

    let mut records = Vec::new();
    for entry in fs::read_dir(sessions_dir).with_context(|| {
        format!(
            "Failed to read sessions directory {}",
            sessions_dir.display()
        )
    })? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read session file {}", path.display()))?;
        let session: Session = serde_json::from_str(&contents)
            .with_context(|| format!("Failed to parse session file {}", path.display()))?;
        let status = if platform::is_process_running(session.pid) {
            SessionStatus::Running
        } else {
            SessionStatus::Stale
        };
        records.push(SessionRecord {
            session,
            status,
            path,
        });
    }

    records.sort_by_key(|record| record.session.started_at);
    Ok(records)
}

pub fn cleanup_stale_sessions(records: &[SessionRecord]) -> Result<()> {
    for record in records {
        if record.status == SessionStatus::Stale {
            fs::remove_file(&record.path).with_context(|| {
                format!(
                    "Failed to remove stale session file {}",
                    record.path.display()
                )
            })?;
        }
    }
    Ok(())
}

pub fn stop_session(sessions_dir: &Path, selector: &str) -> Result<Session> {
    let records = list_sessions(sessions_dir)?;
    let matches: Vec<_> = records
        .iter()
        .filter(|record| {
            record.session.id.to_string().starts_with(selector)
                || record.session.preset_name == selector
        })
        .collect();

    let record = match matches.as_slice() {
        [] => return Err(anyhow!("No known running session matches: {}", selector)),
        [record] => *record,
        _ => return Err(anyhow!("Multiple sessions match: {}", selector)),
    };

    if record.status == SessionStatus::Running {
        platform::terminate_process(record.session.pid)?;
    }
    fs::remove_file(&record.path)
        .with_context(|| format!("Failed to remove session file {}", record.path.display()))?;
    Ok(record.session.clone())
}

fn session_path(sessions_dir: &Path, id: Uuid) -> PathBuf {
    sessions_dir.join(format!("{}.json", id))
}

fn serde_json_pretty<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string_pretty(value).context("Failed to serialize session")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn preset() -> ForwardPreset {
        ForwardPreset {
            name: "jupyter".to_string(),
            host: "server".to_string(),
            local_port: 8888,
            remote_host: "127.0.0.1".to_string(),
            remote_port: 8888,
            bind_address: None,
            extra_args: vec![],
        }
    }

    #[test]
    fn writes_and_reads_session_metadata() {
        let dir = std::env::temp_dir().join(format!("molenest-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let session = Session::new(&preset(), 999_999, "ssh -N".to_string());

        save_session(&session, &dir).unwrap();
        let records = list_sessions(&dir).unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].session.preset_name, "jupyter");

        fs::remove_dir_all(&dir).unwrap();
    }
}
