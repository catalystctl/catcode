//! Local embedding / hashing-sketch recall for memory retrieval (Milestone 4).
//!
//! Default embedder: hashing sketch (no ML dependency). Optional HTTP
//! `embed_endpoint` can supply real vectors later. An on-disk index lives under
//! the memory project hash directory.

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const DIM: usize = 256;
const SYNONYM_MISS_THRESHOLD: f64 = 0.35;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmbeddingIndex {
    pub version: u32,
    pub vectors: HashMap<String, Vec<f32>>,
}

impl Default for EmbeddingIndex {
    fn default() -> Self {
        Self {
            version: 1,
            vectors: HashMap::new(),
        }
    }
}

fn index_path(workspace: &Path) -> PathBuf {
    let hash = crate::memory::project_hash(&workspace.display().to_string());
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".config/catalyst-code/memory")
        .join(hash)
        .join("embed_index.json")
}

pub fn load_index(workspace: &Path) -> EmbeddingIndex {
    let p = index_path(workspace);
    std::fs::read_to_string(p)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_index(workspace: &Path, index: &EmbeddingIndex) {
    let p = index_path(workspace);
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(s) = serde_json::to_string(index) {
        let _ = std::fs::write(p, s);
    }
}

/// Simple hashing sketch: bag-of-hashed-tokens into a fixed DIM vector.
pub fn hash_embed(text: &str) -> Vec<f32> {
    let mut v = vec![0f32; DIM];
    for tok in text.split(|c: char| !c.is_alphanumeric()).filter(|t| t.len() > 1) {
        let t = tok.to_ascii_lowercase();
        let mut h: u64 = 0xcbf29ce484222325;
        for b in t.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        let idx = (h as usize) % DIM;
        let sign = if (h >> 32) & 1 == 0 { 1.0 } else { -1.0 };
        v[idx] += sign;
    }
    // L2 normalize
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    for x in &mut v {
        *x /= norm;
    }
    v
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
    }
    dot
}

/// Upsert memory id → embedding.
pub fn index_memory(workspace: &Path, id: &str, text: &str) {
    let mut idx = load_index(workspace);
    idx.vectors.insert(id.to_string(), hash_embed(text));
    save_index(workspace, &idx);
}

/// Rank memory ids by cosine similarity to the query.
pub fn search(workspace: &Path, query: &str, limit: usize) -> Vec<(String, f32)> {
    let idx = load_index(workspace);
    if idx.vectors.is_empty() {
        return Vec::new();
    }
    let q = hash_embed(query);
    let mut scored: Vec<(String, f32)> = idx
        .vectors
        .iter()
        .map(|(id, v)| (id.clone(), cosine(&q, v)))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(limit);
    scored
}

/// True when recent synonym-miss rate warrants preferring embedding retrieval.
pub fn should_prefer_embeddings(synonym_misses: u64, synonym_hits: u64) -> bool {
    let total = synonym_misses + synonym_hits;
    if total < 4 {
        return false;
    }
    (synonym_misses as f64) / (total as f64) >= SYNONYM_MISS_THRESHOLD
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_embed_is_normalized_and_stable() {
        let a = hash_embed("hello world coding agent");
        let b = hash_embed("hello world coding agent");
        assert_eq!(a.len(), DIM);
        assert!((cosine(&a, &b) - 1.0).abs() < 1e-5);
        let c = hash_embed("completely different topic xyz");
        assert!(cosine(&a, &c) < 0.95);
    }

    #[test]
    fn prefer_embeddings_threshold() {
        assert!(!should_prefer_embeddings(1, 0));
        assert!(should_prefer_embeddings(4, 0));
        assert!(!should_prefer_embeddings(1, 10));
    }

    #[test]
    fn cosine_orthogonal_vectors_return_zero() {
        // Two vectors with zero dot product.
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!((cosine(&a, &b) - 0.0).abs() < 1e-5);
    }

    #[test]
    fn cosine_mismatched_lengths_returns_zero() {
        assert_eq!(cosine(&[1.0, 0.0], &[1.0]), 0.0);
    }

    #[test]
    fn dim_constant_is_256() {
        assert_eq!(DIM, 256);
    }

    #[test]
    fn search_on_empty_index_returns_empty() {
        let tmp = std::env::temp_dir().join(format!("catcode-embed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        // No index file → search returns empty.
        let results = search(&tmp, "any query", 10);
        assert!(results.is_empty());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
