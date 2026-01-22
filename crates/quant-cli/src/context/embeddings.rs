//! Embedding-based semantic search
//!
//! Uses fastembed for local embedding generation and semantic similarity search.
//! This module is optional and requires the `embeddings` feature.

use anyhow::{Context, Result};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, info, warn};

#[cfg(feature = "embeddings")]
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

/// Default embedding model
pub const DEFAULT_MODEL: &str = "all-MiniLM-L6-v2";

/// Embedding vector type
pub type Embedding = Vec<f32>;

/// Cached embedding with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingEntry {
    /// File path
    pub path: PathBuf,
    /// Content hash for invalidation
    pub content_hash: String,
    /// Embedding vector
    pub embedding: Embedding,
}

/// Embedding engine for semantic search
pub struct EmbeddingEngine {
    /// Model used for embeddings
    #[allow(dead_code)]
    model_name: String,
    /// FastEmbed model instance
    #[cfg(feature = "embeddings")]
    model: Option<TextEmbedding>,
    /// Embedding cache
    cache: Arc<RwLock<HashMap<PathBuf, EmbeddingEntry>>>,
    /// Cache file path
    cache_path: PathBuf,
}

impl EmbeddingEngine {
    /// Create a new embedding engine
    pub fn new(model_name: &str, cache_dir: &Path) -> Result<Self> {
        let cache_path = cache_dir.join("embeddings.bin");

        // Load cache if exists
        let cache = if cache_path.exists() {
            match std::fs::read(&cache_path) {
                Ok(data) => {
                    match bincode::deserialize::<HashMap<PathBuf, EmbeddingEntry>>(&data) {
                        Ok(entries) => {
                            debug!(entries = entries.len(), "Loaded embedding cache");
                            Arc::new(RwLock::new(entries))
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to deserialize embedding cache");
                            Arc::new(RwLock::new(HashMap::new()))
                        }
                    }
                }
                Err(e) => {
                    warn!(error = %e, "Failed to read embedding cache");
                    Arc::new(RwLock::new(HashMap::new()))
                }
            }
        } else {
            Arc::new(RwLock::new(HashMap::new()))
        };

        #[cfg(feature = "embeddings")]
        let model = {
            // Initialize the embedding model
            match TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2)) {
                Ok(m) => {
                    info!(model = model_name, "Initialized embedding model");
                    Some(m)
                }
                Err(e) => {
                    warn!(error = %e, "Failed to initialize embedding model");
                    None
                }
            }
        };

        Ok(Self {
            model_name: model_name.to_string(),
            #[cfg(feature = "embeddings")]
            model,
            cache,
            cache_path,
        })
    }

    /// Generate embedding for text
    pub fn embed(&self, text: &str) -> Result<Embedding> {
        #[cfg(feature = "embeddings")]
        {
            if let Some(ref model) = self.model {
                let embeddings = model
                    .embed(vec![text], None)
                    .context("Failed to generate embedding")?;

                if let Some(embedding) = embeddings.into_iter().next() {
                    return Ok(embedding);
                }
            }
        }

        // Fallback: return empty embedding (disables semantic search)
        warn!("Embedding model not available, returning empty embedding");
        Ok(vec![])
    }

    /// Generate embeddings for multiple texts
    pub fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Embedding>> {
        #[cfg(feature = "embeddings")]
        {
            if let Some(ref model) = self.model {
                let embeddings = model
                    .embed(texts.to_vec(), None)
                    .context("Failed to generate embeddings")?;

                return Ok(embeddings);
            }
        }

        // Fallback
        Ok(texts.iter().map(|_| vec![]).collect())
    }

    /// Get or compute embedding for a file
    pub fn get_file_embedding(
        &self,
        path: &Path,
        content: &str,
        content_hash: &str,
    ) -> Result<Embedding> {
        // Check cache
        {
            let cache = self.cache.read();
            if let Some(entry) = cache.get(path) {
                if entry.content_hash == content_hash {
                    return Ok(entry.embedding.clone());
                }
            }
        }

        // Generate new embedding
        let embedding = self.embed(content)?;

        // Cache it
        {
            let mut cache = self.cache.write();
            cache.insert(
                path.to_path_buf(),
                EmbeddingEntry {
                    path: path.to_path_buf(),
                    content_hash: content_hash.to_string(),
                    embedding: embedding.clone(),
                },
            );
        }

        Ok(embedding)
    }

    /// Compute cosine similarity between two embeddings
    pub fn cosine_similarity(a: &Embedding, b: &Embedding) -> f32 {
        if a.is_empty() || b.is_empty() || a.len() != b.len() {
            return 0.0;
        }

        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

        if mag_a == 0.0 || mag_b == 0.0 {
            return 0.0;
        }

        dot / (mag_a * mag_b)
    }

    /// Search for similar files based on query embedding
    pub fn search(&self, query_embedding: &Embedding, top_k: usize) -> Vec<(PathBuf, f32)> {
        let cache = self.cache.read();

        let mut results: Vec<(PathBuf, f32)> = cache
            .iter()
            .map(|(path, entry)| {
                let similarity = Self::cosine_similarity(query_embedding, &entry.embedding);
                (path.clone(), similarity)
            })
            .collect();

        // Sort by similarity descending
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Return top_k results
        results.truncate(top_k);
        results
    }

    /// Save cache to disk
    pub fn save(&self) -> Result<()> {
        let cache = self.cache.read();
        let data = bincode::serialize(&*cache).context("Failed to serialize embedding cache")?;
        std::fs::write(&self.cache_path, data)?;
        debug!(entries = cache.len(), "Saved embedding cache");
        Ok(())
    }

    /// Clear the cache
    pub fn clear(&self) {
        let mut cache = self.cache.write();
        cache.clear();
    }

    /// Check if embedding model is available
    pub fn is_available(&self) -> bool {
        #[cfg(feature = "embeddings")]
        {
            self.model.is_some()
        }
        #[cfg(not(feature = "embeddings"))]
        {
            false
        }
    }

    /// Get cache statistics
    pub fn cache_stats(&self) -> (usize, usize) {
        let cache = self.cache.read();
        let count = cache.len();
        let total_dims: usize = cache.values().map(|e| e.embedding.len()).sum();
        (count, total_dims)
    }
}

impl Drop for EmbeddingEngine {
    fn drop(&mut self) {
        // Try to save cache on drop
        if let Err(e) = self.save() {
            warn!(error = %e, "Failed to save embedding cache on drop");
        }
    }
}

/// Semantic search result
#[derive(Debug, Clone)]
pub struct SemanticSearchResult {
    pub path: PathBuf,
    pub similarity: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let similarity = EmbeddingEngine::cosine_similarity(&a, &b);
        assert!((similarity - 1.0).abs() < 0.001);

        let c = vec![0.0, 1.0, 0.0];
        let similarity = EmbeddingEngine::cosine_similarity(&a, &c);
        assert!(similarity.abs() < 0.001);

        let d = vec![-1.0, 0.0, 0.0];
        let similarity = EmbeddingEngine::cosine_similarity(&a, &d);
        assert!((similarity - (-1.0)).abs() < 0.001);
    }

    #[test]
    fn test_empty_embeddings() {
        let a: Vec<f32> = vec![];
        let b = vec![1.0, 2.0];
        let similarity = EmbeddingEngine::cosine_similarity(&a, &b);
        assert_eq!(similarity, 0.0);
    }
}
