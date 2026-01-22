//! Conversation management and persistence
//!
//! Handles chat history, saving/loading conversations, and session management.

#![allow(dead_code)]

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use llm_core::{ChatMessage, Role};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// A saved conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Conversation {
    /// Unique identifier
    pub id: String,
    /// Human-readable title
    pub title: String,
    /// Model used
    pub model: String,
    /// System prompt (if any)
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Chat messages
    pub messages: Vec<ChatMessage>,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
    /// Last update timestamp
    pub updated_at: DateTime<Utc>,
}

impl Conversation {
    /// Create a new conversation
    pub fn new(model: String, system_prompt: Option<String>) -> Self {
        let id = uuid_v4();
        let now = Utc::now();

        Self {
            id,
            title: "New conversation".to_string(),
            model,
            system_prompt,
            messages: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    /// Add a message to the conversation
    pub fn add_message(&mut self, message: ChatMessage) {
        self.messages.push(message);
        self.updated_at = Utc::now();

        // Update title from first user message
        if self.title == "New conversation" {
            if let Some(first_user) = self.messages.iter().find(|m| m.role == Role::User) {
                self.title = truncate_title(&first_user.content);
            }
        }
    }

    /// Get messages with system prompt prepended
    pub fn messages_with_system(&self) -> Vec<ChatMessage> {
        let mut messages = Vec::new();

        if let Some(ref sys) = self.system_prompt {
            messages.push(ChatMessage::system(sys.clone()));
        }

        messages.extend(self.messages.clone());
        messages
    }

    /// Clear conversation messages
    pub fn clear(&mut self) {
        self.messages.clear();
        self.updated_at = Utc::now();
    }

    /// Get message count
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

/// Manages conversation storage
pub struct ConversationStore {
    /// Directory where conversations are stored
    dir: PathBuf,
}

impl ConversationStore {
    /// Create a new conversation store
    pub fn new() -> Result<Self> {
        let dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("quant")
            .join("conversations");

        fs::create_dir_all(&dir).context("Failed to create conversations directory")?;

        Ok(Self { dir })
    }

    /// Save a conversation
    pub fn save(&self, conversation: &Conversation) -> Result<PathBuf> {
        let path = self.dir.join(format!("{}.json", conversation.id));
        let content = serde_json::to_string_pretty(conversation)?;
        fs::write(&path, content)?;
        Ok(path)
    }

    /// Load a conversation by ID
    pub fn load(&self, id: &str) -> Result<Conversation> {
        let path = self.dir.join(format!("{}.json", id));
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Conversation not found: {}", id))?;
        serde_json::from_str(&content).context("Failed to parse conversation")
    }

    /// Load a conversation by filename (without extension)
    pub fn load_by_name(&self, name: &str) -> Result<Conversation> {
        // Try exact match first
        let path = self.dir.join(format!("{}.json", name));
        if path.exists() {
            let content = fs::read_to_string(&path)?;
            return serde_json::from_str(&content).context("Failed to parse conversation");
        }

        // Try partial match
        let entries = fs::read_dir(&self.dir)?;
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let file_str = file_name.to_string_lossy();
            if file_str.starts_with(name) && file_str.ends_with(".json") {
                let content = fs::read_to_string(entry.path())?;
                return serde_json::from_str(&content).context("Failed to parse conversation");
            }
        }

        anyhow::bail!("Conversation not found: {}", name)
    }

    /// List all conversations
    pub fn list(&self) -> Result<Vec<ConversationSummary>> {
        let mut summaries = Vec::new();

        for entry in fs::read_dir(&self.dir)?.flatten() {
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(conv) = serde_json::from_str::<Conversation>(&content) {
                        summaries.push(ConversationSummary {
                            id: conv.id,
                            title: conv.title,
                            model: conv.model,
                            message_count: conv.messages.len(),
                            updated_at: conv.updated_at,
                        });
                    }
                }
            }
        }

        // Sort by updated_at descending
        summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));

        Ok(summaries)
    }

    /// Delete a conversation
    pub fn delete(&self, id: &str) -> Result<()> {
        let path = self.dir.join(format!("{}.json", id));
        fs::remove_file(&path).context("Failed to delete conversation")?;
        Ok(())
    }

    /// Get the conversations directory path
    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// Summary of a conversation for listing
#[derive(Debug, Clone, Serialize)]
pub struct ConversationSummary {
    pub id: String,
    pub title: String,
    pub model: String,
    pub message_count: usize,
    pub updated_at: DateTime<Utc>,
}

/// REPL input history manager
pub struct InputHistory {
    /// Path to history file
    path: PathBuf,
}

impl InputHistory {
    /// Create a new history manager
    pub fn new() -> Result<Self> {
        let dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("quant");

        fs::create_dir_all(&dir)?;

        Ok(Self {
            path: dir.join("history"),
        })
    }

    /// Get the history file path
    pub fn path(&self) -> &Path {
        &self.path
    }
}

// Helper functions

/// Generate a simple UUID v4
fn uuid_v4() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();

    let random: u64 = (timestamp as u64) ^ (std::process::id() as u64);

    format!(
        "{:08x}-{:04x}-4{:03x}-{:04x}-{:012x}",
        (timestamp >> 96) as u32,
        (timestamp >> 80) as u16,
        (timestamp >> 64) as u16 & 0x0fff,
        ((random >> 48) as u16 & 0x3fff) | 0x8000,
        random & 0xffffffffffff
    )
}

/// Truncate content to a reasonable title length
fn truncate_title(content: &str) -> String {
    let first_line = content.lines().next().unwrap_or(content);
    let cleaned: String = first_line
        .chars()
        .filter(|c| !c.is_control())
        .take(50)
        .collect();

    if cleaned.len() < first_line.len() {
        format!("{}...", cleaned.trim())
    } else {
        cleaned.trim().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_conversation_new() {
        let conv = Conversation::new("test-model".to_string(), None);
        assert!(!conv.id.is_empty());
        assert_eq!(conv.model, "test-model");
        assert!(conv.messages.is_empty());
    }

    #[test]
    fn test_add_message() {
        let mut conv = Conversation::new("test-model".to_string(), None);
        conv.add_message(ChatMessage::user("Hello!"));
        assert_eq!(conv.messages.len(), 1);
        assert_eq!(conv.title, "Hello!");
    }

    #[test]
    fn test_truncate_title() {
        let long = "This is a very long message that should be truncated because it exceeds the maximum title length";
        let title = truncate_title(long);
        assert!(title.len() <= 55); // 50 chars + "..."
    }
}
