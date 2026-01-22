//! Session persistence for conversation history
//!
//! Saves and loads conversation sessions to allow resuming work.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use llm_core::ChatMessageWithTools;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tracing::{debug, info, warn};

/// Unique session identifier
pub type SessionId = String;

/// A saved conversation session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique session ID
    pub id: SessionId,
    /// Human-readable name (auto-generated or user-provided)
    pub name: String,
    /// When the session was created
    pub created_at: DateTime<Utc>,
    /// When the session was last updated
    pub updated_at: DateTime<Utc>,
    /// Project root path (if discovered)
    pub project_root: Option<PathBuf>,
    /// Model used
    pub model: String,
    /// Conversation messages
    pub messages: Vec<ChatMessageWithTools>,
    /// Summary of what was accomplished (auto-generated)
    pub summary: Option<String>,
}

impl Session {
    /// Create a new session
    pub fn new(model: impl Into<String>, project_root: Option<PathBuf>) -> Self {
        let id = generate_session_id();
        let now = Utc::now();

        Self {
            id,
            name: format!("Session {}", now.format("%Y-%m-%d %H:%M")),
            created_at: now,
            updated_at: now,
            project_root,
            model: model.into(),
            messages: Vec::new(),
            summary: None,
        }
    }

    /// Add a message to the session
    pub fn add_message(&mut self, message: ChatMessageWithTools) {
        self.messages.push(message);
        self.updated_at = Utc::now();
    }

    /// Set session name
    pub fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
        self.updated_at = Utc::now();
    }

    /// Set summary
    pub fn set_summary(&mut self, summary: impl Into<String>) {
        self.summary = Some(summary.into());
        self.updated_at = Utc::now();
    }

    /// Get message count (excluding system messages)
    pub fn message_count(&self) -> usize {
        self.messages
            .iter()
            .filter(|m| m.role != llm_core::Role::System)
            .count()
    }
}

/// Session store for saving and loading sessions
pub struct SessionStore {
    /// Base directory for session storage
    base_dir: PathBuf,
}

impl SessionStore {
    /// Create a new session store
    pub fn new() -> Result<Self> {
        let base_dir = get_sessions_dir()?;
        fs::create_dir_all(&base_dir).context("Failed to create sessions directory")?;

        Ok(Self { base_dir })
    }

    /// Save a session to disk
    pub fn save(&self, session: &Session) -> Result<PathBuf> {
        let path = self.session_path(&session.id);

        let json = serde_json::to_string_pretty(session)
            .context("Failed to serialize session")?;

        fs::write(&path, json).context("Failed to write session file")?;

        info!(session_id = %session.id, path = %path.display(), "Saved session");
        Ok(path)
    }

    /// Load a session by ID
    pub fn load(&self, id: &str) -> Result<Session> {
        let path = self.session_path(id);

        if !path.exists() {
            anyhow::bail!("Session not found: {}", id);
        }

        let json = fs::read_to_string(&path).context("Failed to read session file")?;
        let session: Session = serde_json::from_str(&json).context("Failed to parse session")?;

        debug!(session_id = %session.id, messages = session.messages.len(), "Loaded session");
        Ok(session)
    }

    /// List all sessions, sorted by updated_at (most recent first)
    pub fn list(&self) -> Result<Vec<SessionSummary>> {
        let mut sessions = Vec::new();

        if let Ok(entries) = fs::read_dir(&self.base_dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().map_or(false, |e| e == "json") {
                    match self.load_summary(&path) {
                        Ok(summary) => sessions.push(summary),
                        Err(e) => warn!(path = %path.display(), error = %e, "Failed to load session summary"),
                    }
                }
            }
        }

        // Sort by updated_at descending
        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        Ok(sessions)
    }

    /// Delete a session
    pub fn delete(&self, id: &str) -> Result<()> {
        let path = self.session_path(id);

        if !path.exists() {
            anyhow::bail!("Session not found: {}", id);
        }

        fs::remove_file(&path).context("Failed to delete session file")?;
        info!(session_id = %id, "Deleted session");
        Ok(())
    }

    /// Find sessions by project root
    pub fn find_by_project(&self, project_root: &PathBuf) -> Result<Vec<SessionSummary>> {
        let all = self.list()?;
        let canonical = project_root.canonicalize().ok();

        Ok(all
            .into_iter()
            .filter(|s| {
                s.project_root.as_ref().and_then(|p| p.canonicalize().ok()) == canonical
            })
            .collect())
    }

    /// Get the most recent session for a project
    pub fn latest_for_project(&self, project_root: &PathBuf) -> Result<Option<Session>> {
        let sessions = self.find_by_project(project_root)?;
        if let Some(summary) = sessions.first() {
            Ok(Some(self.load(&summary.id)?))
        } else {
            Ok(None)
        }
    }

    fn session_path(&self, id: &str) -> PathBuf {
        self.base_dir.join(format!("{}.json", id))
    }

    fn load_summary(&self, path: &PathBuf) -> Result<SessionSummary> {
        let json = fs::read_to_string(path)?;
        let session: Session = serde_json::from_str(&json)?;

        let message_count = session.message_count();
        Ok(SessionSummary {
            id: session.id,
            name: session.name,
            created_at: session.created_at,
            updated_at: session.updated_at,
            project_root: session.project_root,
            model: session.model,
            message_count,
            summary: session.summary,
        })
    }
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new().expect("Failed to create session store")
    }
}

/// Lightweight summary of a session for listing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: SessionId,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub project_root: Option<PathBuf>,
    pub model: String,
    pub message_count: usize,
    pub summary: Option<String>,
}

impl SessionSummary {
    /// Format as a short one-line description
    pub fn short_description(&self) -> String {
        let age = format_age(&self.updated_at);
        let summary = self.summary.as_deref().unwrap_or(&self.name);
        let truncated = if summary.len() > 50 {
            format!("{}...", &summary[..47])
        } else {
            summary.to_string()
        };

        format!(
            "{} ({} msgs, {}, {})",
            self.id,
            self.message_count,
            self.model,
            age
        )
    }
}

/// Get the sessions directory
fn get_sessions_dir() -> Result<PathBuf> {
    let data_dir = dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .ok_or_else(|| anyhow::anyhow!("Could not find data directory"))?;

    Ok(data_dir.join("quant").join("sessions"))
}

/// Generate a unique session ID
fn generate_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();

    // Use timestamp + random suffix for uniqueness
    let random: u32 = rand_u32();
    format!("{:x}-{:04x}", timestamp, random & 0xFFFF)
}

/// Simple random u32 using system time as seed
fn rand_u32() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    nanos.wrapping_mul(1664525).wrapping_add(1013904223)
}

/// Format a timestamp as relative age
fn format_age(dt: &DateTime<Utc>) -> String {
    let now = Utc::now();
    let duration = now.signed_duration_since(*dt);

    if duration.num_minutes() < 1 {
        "just now".to_string()
    } else if duration.num_hours() < 1 {
        format!("{}m ago", duration.num_minutes())
    } else if duration.num_days() < 1 {
        format!("{}h ago", duration.num_hours())
    } else if duration.num_days() < 7 {
        format!("{}d ago", duration.num_days())
    } else {
        dt.format("%Y-%m-%d").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_core::Role;
    use tempfile::TempDir;

    fn create_test_store() -> (SessionStore, TempDir) {
        let dir = TempDir::new().unwrap();
        let store = SessionStore {
            base_dir: dir.path().to_path_buf(),
        };
        (store, dir)
    }

    #[test]
    fn test_session_creation() {
        let session = Session::new("test-model", None);
        assert!(!session.id.is_empty());
        assert_eq!(session.model, "test-model");
        assert!(session.messages.is_empty());
    }

    #[test]
    fn test_session_save_load() {
        let (store, _dir) = create_test_store();

        let mut session = Session::new("test-model", None);
        session.add_message(ChatMessageWithTools {
            role: Role::User,
            content: "Hello".to_string(),
            tool_calls: None,
            tool_call_id: None,
        });

        let path = store.save(&session).unwrap();
        assert!(path.exists());

        let loaded = store.load(&session.id).unwrap();
        assert_eq!(loaded.id, session.id);
        assert_eq!(loaded.messages.len(), 1);
    }

    #[test]
    fn test_session_list() {
        let (store, _dir) = create_test_store();

        // Create multiple sessions
        for i in 0..3 {
            let mut session = Session::new("test-model", None);
            session.set_name(format!("Session {}", i));
            store.save(&session).unwrap();
        }

        let list = store.list().unwrap();
        assert_eq!(list.len(), 3);
    }

    #[test]
    fn test_session_delete() {
        let (store, _dir) = create_test_store();

        let session = Session::new("test-model", None);
        store.save(&session).unwrap();

        assert!(store.load(&session.id).is_ok());
        store.delete(&session.id).unwrap();
        assert!(store.load(&session.id).is_err());
    }

    #[test]
    fn test_message_count() {
        let mut session = Session::new("test-model", None);

        // System message shouldn't count
        session.add_message(ChatMessageWithTools {
            role: Role::System,
            content: "System".to_string(),
            tool_calls: None,
            tool_call_id: None,
        });

        // User and assistant should count
        session.add_message(ChatMessageWithTools {
            role: Role::User,
            content: "User".to_string(),
            tool_calls: None,
            tool_call_id: None,
        });
        session.add_message(ChatMessageWithTools {
            role: Role::Assistant,
            content: "Assistant".to_string(),
            tool_calls: None,
            tool_call_id: None,
        });

        assert_eq!(session.message_count(), 2);
    }

    #[test]
    fn test_generate_session_id() {
        let id1 = generate_session_id();
        let id2 = generate_session_id();
        // IDs should be unique (extremely unlikely to collide)
        assert_ne!(id1, id2);
    }
}
