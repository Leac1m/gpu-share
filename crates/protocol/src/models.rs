//! Model catalog entry shape (spec §6.5) and the VRAM estimate that decides
//! the "fits your GPU" badge. See roadmap Phase 6.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub file_url: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub quantization: String,
    pub license: String,
    pub default_context: u32,
    /// Shape for the VRAM estimate until the GGUF header parser (`P6-T5`) reads it from the
    /// file itself. Fields below are additions to the spec §6.5 example; all optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<ModelShape>,
    /// Estimated tokens per second on the reference card (RTX A2000), for display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tps_estimate: Option<u32>,
    /// Extra `llama-server` arguments this model needs (e.g. turning Qwen3's thinking off).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub engine_args: Vec<String>,
    /// Tiny models for development; hidden in release builds.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dev_only: bool,
}

impl CatalogEntry {
    /// The file name the model is saved under in the app's models folder.
    pub fn file_name(&self) -> String {
        format!("{}.gguf", self.id)
    }
}

/// The handful of GGUF header fields the VRAM estimate needs (spec §6.5).
/// Reading these out of a real file is `P6-T5`; this struct is the stable
/// interface between that parser and the estimate below, so the estimate
/// can be tested without a real GGUF file.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ModelShape {
    pub layers: u32,
    pub kv_heads: u32,
    pub head_dim: u32,
}

/// Bytes used per KV cache value. llama.cpp defaults to fp16 K/V.
const KV_CACHE_BYTES_PER_VALUE: u64 = 2;
/// Fixed overhead the spec allows for the runtime itself (spec §6.5).
const OVERHEAD_BYTES: u64 = 512 * 1024 * 1024;

/// `VRAM ≈ model file size + KV cache + ~0.5 GB overhead`, per spec
/// §6.5. Kept as a free function of plain numbers (no file I/O, no GPU) so
/// it stays a one-line unit test and can be pinned against the real
/// measurements `P5-T11` takes on the A2000.
pub fn estimate_vram_bytes(file_size_bytes: u64, shape: ModelShape, context: u32) -> u64 {
    let kv_cache = 2u64
        * shape.layers as u64
        * context as u64
        * shape.kv_heads as u64
        * shape.head_dim as u64
        * KV_CACHE_BYTES_PER_VALUE;
    file_size_bytes + kv_cache + OVERHEAD_BYTES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spec_example_entry_still_parses() {
        let e: CatalogEntry = serde_json::from_str(include_str!("../tests/fixtures/catalog_entry.json")).unwrap();
        assert_eq!(e.default_context, 4096);
        assert_eq!(e.shape, None);
        assert!(!e.dev_only);
    }

    /// Rough shape of a 7-8B model, used only to sanity-check the formula's
    /// order of magnitude until `P5-T11` supplies real numbers to pin
    /// against.
    fn small_model_shape() -> ModelShape {
        ModelShape { layers: 32, kv_heads: 8, head_dim: 128 }
    }

    #[test]
    fn a_4gb_model_at_4k_context_fits_comfortably_under_the_6gb_budget() {
        let four_gb = 4 * 1024 * 1024 * 1024;
        let estimate = estimate_vram_bytes(four_gb, small_model_shape(), 4096);
        let six_gb = 6 * 1024 * 1024 * 1024;
        assert!(
            estimate < six_gb,
            "estimate {estimate} should fit the spec's proposed 6 GB budget"
        );
    }

    #[test]
    fn longer_context_increases_the_estimate() {
        let file_size = 4 * 1024 * 1024 * 1024;
        let shape = small_model_shape();
        let short = estimate_vram_bytes(file_size, shape, 2048);
        let long = estimate_vram_bytes(file_size, shape, 8192);
        assert!(long > short, "a longer context must never look cheaper");
    }

    #[test]
    fn zero_context_is_just_file_size_plus_overhead() {
        let file_size = 1024;
        let estimate = estimate_vram_bytes(file_size, small_model_shape(), 0);
        assert_eq!(estimate, file_size + OVERHEAD_BYTES);
    }
}
