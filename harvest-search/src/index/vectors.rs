use anyhow::Result;
use async_trait::async_trait;

use crate::domain::UnitId;
use crate::index::bm25::top_k;
use crate::index::corpus::Unit;
use crate::index::tokenizer::tokens;

#[async_trait]
pub trait Embedder: Send + Sync {
    fn model(&self) -> &str;
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

pub struct HashingEmbedder {
    dims: usize,
    model: String,
}

impl HashingEmbedder {
    pub fn new(dims: usize) -> Self {
        Self {
            dims,
            model: format!("hashing-v1-{dims}"),
        }
    }

    pub fn embed_one(&self, text: &str) -> Vec<f32> {
        let mut vector = vec![0f32; self.dims];
        let toks = tokens(text);
        for window in toks.windows(2) {
            add_feature(&mut vector, &format!("{} {}", window[0], window[1]), 0.5);
        }
        for t in &toks {
            add_feature(&mut vector, t, 1.0);
            for gram in trigrams(t) {
                add_feature(&mut vector, &gram, 0.25);
            }
        }
        normalize(&mut vector);
        vector
    }
}

impl Default for HashingEmbedder {
    fn default() -> Self {
        Self::new(256)
    }
}

#[async_trait]
impl Embedder for HashingEmbedder {
    fn model(&self) -> &str {
        &self.model
    }

    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| self.embed_one(t)).collect())
    }
}

fn trigrams(token: &str) -> Vec<String> {
    let chars: Vec<char> = format!("#{token}#").chars().collect();
    chars.windows(3).map(|w| w.iter().collect()).collect()
}

fn fnv1a(text: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

fn add_feature(vector: &mut [f32], feature: &str, weight: f32) {
    let hash = fnv1a(feature);
    let index = (hash % vector.len() as u64) as usize;
    let sign = if (hash >> 63) == 0 { 1.0 } else { -1.0 };
    vector[index] += sign * weight;
}

pub fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in vector.iter_mut() {
            *v /= norm;
        }
    }
}

pub fn embedding_text(unit: &Unit) -> String {
    let body: String = unit.body.chars().take(1_500).collect();
    format!(
        "{} {} {} {} {}",
        unit.qualified_name, unit.signature, unit.docstring, unit.summary, body
    )
}

#[derive(Clone, Debug, Default)]
pub struct VectorIndex {
    model: String,
    dims: usize,
    data: Vec<f32>,
    present: Vec<bool>,
}

impl VectorIndex {
    pub fn from_vectors(model: impl Into<String>, vectors: Vec<Option<Vec<f32>>>) -> Self {
        let dims = vectors.iter().flatten().map(Vec::len).next().unwrap_or(0);
        let mut data = Vec::with_capacity(vectors.len() * dims);
        let mut present = Vec::with_capacity(vectors.len());
        for vector in vectors {
            match vector {
                Some(mut v) if v.len() == dims && dims > 0 => {
                    normalize(&mut v);
                    data.extend_from_slice(&v);
                    present.push(true);
                }
                _ => {
                    data.extend(std::iter::repeat_n(0.0, dims));
                    present.push(false);
                }
            }
        }
        Self {
            model: model.into(),
            dims,
            data,
            present,
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn len(&self) -> usize {
        self.present.iter().filter(|p| **p).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn search(&self, query: &[f32], k: usize) -> Vec<(UnitId, f32)> {
        if query.len() != self.dims || self.dims == 0 {
            return Vec::new();
        }
        let mut q = query.to_vec();
        normalize(&mut q);
        let scored = self
            .data
            .chunks_exact(self.dims)
            .zip(&self.present)
            .enumerate()
            .filter(|(_, (_, present))| **present)
            .map(|(i, (v, _))| (i as UnitId, v.iter().zip(&q).map(|(a, b)| a * b).sum()))
            .collect();
        top_k(scored, k)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashing_embeddings_are_unit_length_and_deterministic() {
        let embedder = HashingEmbedder::new(64);
        let a = embedder.embed_one("initialize_connection for volume drivers");
        let b = embedder.embed_one("initialize_connection for volume drivers");
        assert_eq!(a, b);
        let norm: f32 = a.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4);
        assert_eq!(embedder.model(), "hashing-v1-64");
    }

    #[test]
    fn related_texts_score_closer_than_unrelated_ones() {
        let embedder = HashingEmbedder::default();
        let index = VectorIndex::from_vectors(
            embedder.model(),
            vec![
                Some(embedder.embed_one("attach a volume to an instance")),
                Some(embedder.embed_one("parse the yaml configuration file")),
                None,
            ],
        );
        assert_eq!(index.len(), 2);
        let hits = index.search(&embedder.embed_one("volume attachment"), 3);
        assert_eq!(hits[0].0, 0);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn mismatched_dimensions_return_nothing() {
        let index = VectorIndex::from_vectors("m", vec![Some(vec![1.0, 0.0])]);
        assert!(index.search(&[1.0, 0.0, 0.0], 5).is_empty());
    }
}
