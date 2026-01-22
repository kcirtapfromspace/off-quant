//! File index with caching
//!
//! Maintains metadata about files in the project for efficient context selection.

use anyhow::{Context, Result};
use dashmap::DashMap;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;
use tracing::debug;

use super::tokenizer::count_tokens;

/// File metadata for indexing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    /// File path relative to project root
    pub path: PathBuf,
    /// File size in bytes
    pub size: u64,
    /// Last modified timestamp
    pub modified: u64,
    /// Token count (cached)
    pub token_count: usize,
    /// Content hash for invalidation
    pub content_hash: String,
    /// File extension
    pub extension: String,
}

impl FileMetadata {
    /// Create metadata from a file path
    pub fn from_path(path: &Path, project_root: &Path) -> Result<Self> {
        let metadata = fs::metadata(path).context("Failed to read file metadata")?;
        let content = fs::read_to_string(path).context("Failed to read file content")?;

        let modified = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let content_hash = compute_hash(&content);
        let token_count = count_tokens(&content);
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_string();

        let rel_path = path
            .strip_prefix(project_root)
            .unwrap_or(path)
            .to_path_buf();

        Ok(Self {
            path: rel_path,
            size: metadata.len(),
            modified,
            token_count,
            content_hash,
            extension,
        })
    }

    /// Check if metadata is stale (file changed)
    pub fn is_stale(&self, current_modified: u64, current_hash: &str) -> bool {
        self.modified != current_modified || self.content_hash != current_hash
    }
}

/// Compute SHA256 hash of content
fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// File index for the project
pub struct FileIndex {
    /// Project root directory
    project_root: PathBuf,
    /// Cache of file metadata by path
    cache: Arc<DashMap<PathBuf, FileMetadata>>,
    /// Path to persist cache
    cache_path: PathBuf,
    /// Last full scan time
    last_scan: RwLock<Option<u64>>,
}

impl FileIndex {
    /// Create a new file index for the given project root
    pub fn new(project_root: PathBuf) -> Result<Self> {
        let cache_dir = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from(".cache"))
            .join("quant");
        fs::create_dir_all(&cache_dir)?;

        // Generate cache path based on project root hash
        let project_hash = compute_hash(&project_root.to_string_lossy());
        let cache_path = cache_dir.join(format!("index_{}.json", &project_hash[..16]));

        let cache = Arc::new(DashMap::new());

        // Load existing cache if available
        if cache_path.exists() {
            if let Ok(content) = fs::read_to_string(&cache_path) {
                if let Ok(entries) = serde_json::from_str::<HashMap<PathBuf, FileMetadata>>(&content)
                {
                    for (path, meta) in entries {
                        cache.insert(path, meta);
                    }
                    debug!(entries = cache.len(), "Loaded file index cache");
                }
            }
        }

        Ok(Self {
            project_root,
            cache,
            cache_path,
            last_scan: RwLock::new(None),
        })
    }

    /// Get metadata for a file, updating cache if needed
    pub fn get(&self, path: &Path) -> Option<FileMetadata> {
        let abs_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.project_root.join(path)
        };

        let rel_path = abs_path
            .strip_prefix(&self.project_root)
            .unwrap_or(&abs_path)
            .to_path_buf();

        // Check cache
        if let Some(cached) = self.cache.get(&rel_path) {
            // Validate cache entry
            if let Ok(metadata) = fs::metadata(&abs_path) {
                let modified = metadata
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);

                // Quick check: if modified time matches, cache is likely valid
                if cached.modified == modified {
                    return Some(cached.clone());
                }

                // Full check: compare hash
                if let Ok(content) = fs::read_to_string(&abs_path) {
                    let current_hash = compute_hash(&content);
                    if !cached.is_stale(modified, &current_hash) {
                        return Some(cached.clone());
                    }
                }
            }
        }

        // Cache miss or stale - update
        if let Ok(meta) = FileMetadata::from_path(&abs_path, &self.project_root) {
            self.cache.insert(rel_path, meta.clone());
            return Some(meta);
        }

        None
    }

    /// Get metadata for multiple files
    pub fn get_many(&self, paths: &[PathBuf]) -> Vec<FileMetadata> {
        paths.iter().filter_map(|p| self.get(p)).collect()
    }

    /// Update index for a specific file
    pub fn update(&self, path: &Path) -> Result<Option<FileMetadata>> {
        let abs_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.project_root.join(path)
        };

        let rel_path = abs_path
            .strip_prefix(&self.project_root)
            .unwrap_or(&abs_path)
            .to_path_buf();

        match FileMetadata::from_path(&abs_path, &self.project_root) {
            Ok(meta) => {
                self.cache.insert(rel_path, meta.clone());
                Ok(Some(meta))
            }
            Err(_) => {
                self.cache.remove(&rel_path);
                Ok(None)
            }
        }
    }

    /// Remove a file from the index
    pub fn remove(&self, path: &Path) {
        let rel_path = path
            .strip_prefix(&self.project_root)
            .unwrap_or(path)
            .to_path_buf();
        self.cache.remove(&rel_path);
    }

    /// Save cache to disk
    pub fn save(&self) -> Result<()> {
        let entries: HashMap<PathBuf, FileMetadata> = self
            .cache
            .iter()
            .map(|e| (e.key().clone(), e.value().clone()))
            .collect();

        let content = serde_json::to_string_pretty(&entries)?;
        fs::write(&self.cache_path, content)?;

        debug!(entries = entries.len(), path = ?self.cache_path, "Saved file index cache");
        Ok(())
    }

    /// Clear the cache
    pub fn clear(&self) {
        self.cache.clear();
        *self.last_scan.write() = None;
    }

    /// Get cache statistics
    pub fn stats(&self) -> IndexStats {
        let total_files = self.cache.len();
        let total_tokens: usize = self.cache.iter().map(|e| e.token_count).sum();
        let total_size: u64 = self.cache.iter().map(|e| e.size).sum();

        IndexStats {
            total_files,
            total_tokens,
            total_size,
        }
    }

    /// Get all indexed files
    pub fn all_files(&self) -> Vec<FileMetadata> {
        self.cache.iter().map(|e| e.value().clone()).collect()
    }

    /// Get files by extension
    pub fn files_by_extension(&self, ext: &str) -> Vec<FileMetadata> {
        self.cache
            .iter()
            .filter(|e| e.extension == ext)
            .map(|e| e.value().clone())
            .collect()
    }

    /// Get files matching a size constraint
    pub fn files_under_size(&self, max_bytes: u64) -> Vec<FileMetadata> {
        self.cache
            .iter()
            .filter(|e| e.size <= max_bytes)
            .map(|e| e.value().clone())
            .collect()
    }

    /// Get files matching a token constraint
    pub fn files_under_tokens(&self, max_tokens: usize) -> Vec<FileMetadata> {
        self.cache
            .iter()
            .filter(|e| e.token_count <= max_tokens)
            .map(|e| e.value().clone())
            .collect()
    }
}

/// Statistics about the file index
#[derive(Debug, Clone)]
pub struct IndexStats {
    pub total_files: usize,
    pub total_tokens: usize,
    pub total_size: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_file_metadata_from_path() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.rs");

        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "fn main() {{ println!(\"Hello\"); }}").unwrap();

        let meta = FileMetadata::from_path(&file_path, dir.path()).unwrap();

        assert_eq!(meta.path, PathBuf::from("test.rs"));
        assert_eq!(meta.extension, "rs");
        assert!(meta.token_count > 0);
        assert!(!meta.content_hash.is_empty());
    }

    #[test]
    fn test_file_index_get_and_cache() {
        let dir = TempDir::new().unwrap();
        let file_path = dir.path().join("test.rs");

        let mut file = File::create(&file_path).unwrap();
        writeln!(file, "fn test() {{}}").unwrap();

        let index = FileIndex::new(dir.path().to_path_buf()).unwrap();

        // First get - populates cache
        let meta1 = index.get(&file_path).unwrap();
        assert_eq!(meta1.extension, "rs");

        // Second get - from cache
        let meta2 = index.get(&file_path).unwrap();
        assert_eq!(meta1.content_hash, meta2.content_hash);
    }

    #[test]
    fn test_compute_hash() {
        let hash1 = compute_hash("hello");
        let hash2 = compute_hash("hello");
        let hash3 = compute_hash("world");

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }
}
