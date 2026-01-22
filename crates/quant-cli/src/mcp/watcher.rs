//! Hot-reload support for QUANT.md changes
//!
//! Watches for changes to QUANT.md and triggers MCP server reconfiguration.

use anyhow::Result;
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;
use tracing::{debug, info, warn};

/// Event types for configuration changes
#[derive(Debug, Clone)]
pub enum ConfigChangeEvent {
    /// QUANT.md was modified
    QuantMdModified(PathBuf),
    /// QUANT.md was created
    QuantMdCreated(PathBuf),
    /// QUANT.md was deleted
    QuantMdDeleted(PathBuf),
}

/// Watcher for configuration file changes
pub struct ConfigWatcher {
    #[allow(dead_code)]
    watcher: RecommendedWatcher,
    receiver: Receiver<Result<Event, notify::Error>>,
    quant_md_path: Option<PathBuf>,
}

impl ConfigWatcher {
    /// Create a new config watcher for a project directory
    pub fn new(project_root: &Path) -> Result<Self> {
        let (tx, rx) = channel();

        let watcher = RecommendedWatcher::new(
            move |result| {
                let _ = tx.send(result);
            },
            Config::default().with_poll_interval(Duration::from_secs(2)),
        )?;

        let quant_md_path = Self::find_quant_md(project_root);

        Ok(Self {
            watcher,
            receiver: rx,
            quant_md_path,
        })
    }

    /// Start watching the configuration file
    pub fn start(&mut self) -> Result<()> {
        if let Some(ref path) = self.quant_md_path {
            self.watcher.watch(path, RecursiveMode::NonRecursive)?;
            info!(path = ?path, "Started watching QUANT.md for changes");
        } else {
            debug!("No QUANT.md found to watch");
        }
        Ok(())
    }

    /// Stop watching
    pub fn stop(&mut self) -> Result<()> {
        if let Some(ref path) = self.quant_md_path {
            self.watcher.unwatch(path)?;
            info!("Stopped watching QUANT.md");
        }
        Ok(())
    }

    /// Check for pending change events (non-blocking)
    pub fn poll_events(&self) -> Vec<ConfigChangeEvent> {
        let mut events = Vec::new();

        while let Ok(result) = self.receiver.try_recv() {
            match result {
                Ok(event) => {
                    if let Some(change_event) = self.process_event(event) {
                        events.push(change_event);
                    }
                }
                Err(e) => {
                    warn!(error = %e, "File watcher error");
                }
            }
        }

        events
    }

    /// Wait for the next change event (blocking)
    pub fn wait_for_event(&self) -> Option<ConfigChangeEvent> {
        match self.receiver.recv() {
            Ok(Ok(event)) => self.process_event(event),
            Ok(Err(e)) => {
                warn!(error = %e, "File watcher error");
                None
            }
            Err(_) => None,
        }
    }

    /// Process a notify event into a config change event
    fn process_event(&self, event: Event) -> Option<ConfigChangeEvent> {
        let quant_md = self.quant_md_path.as_ref()?;

        // Check if this event is for QUANT.md
        let is_quant_md = event
            .paths
            .iter()
            .any(|p| p.ends_with("QUANT.md") || p.ends_with("quant.md"));

        if !is_quant_md {
            return None;
        }

        match event.kind {
            EventKind::Modify(_) => {
                info!("QUANT.md modified");
                Some(ConfigChangeEvent::QuantMdModified(quant_md.clone()))
            }
            EventKind::Create(_) => {
                info!("QUANT.md created");
                Some(ConfigChangeEvent::QuantMdCreated(quant_md.clone()))
            }
            EventKind::Remove(_) => {
                info!("QUANT.md deleted");
                Some(ConfigChangeEvent::QuantMdDeleted(quant_md.clone()))
            }
            _ => None,
        }
    }

    /// Find QUANT.md in the project directory
    fn find_quant_md(root: &Path) -> Option<PathBuf> {
        let candidates = ["QUANT.md", "quant.md"];

        for candidate in candidates {
            let path = root.join(candidate);
            if path.exists() {
                return Some(path);
            }
        }

        None
    }

    /// Check if QUANT.md exists
    pub fn has_quant_md(&self) -> bool {
        self.quant_md_path.is_some()
    }

    /// Get the path to QUANT.md
    pub fn quant_md_path(&self) -> Option<&Path> {
        self.quant_md_path.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_find_quant_md() {
        let dir = TempDir::new().unwrap();
        let quant_path = dir.path().join("QUANT.md");
        File::create(&quant_path).unwrap();

        let found = ConfigWatcher::find_quant_md(dir.path());
        assert!(found.is_some());
        assert_eq!(found.unwrap(), quant_path);
    }

    #[test]
    fn test_no_quant_md() {
        let dir = TempDir::new().unwrap();
        let found = ConfigWatcher::find_quant_md(dir.path());
        assert!(found.is_none());
    }
}
