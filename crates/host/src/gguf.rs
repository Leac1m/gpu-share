//! Reading the few GGUF header fields the VRAM estimate needs (`P6-T5`, spec §6.5): layer count,
//! KV heads and head size.
//!
//! Model files are data from the internet, so this parser assumes the worst: every length is
//! checked against hard caps before anything is allocated, it reads at most [`MAX_HEADER_BYTES`],
//! and malformed input is an error, never a panic (roadmap suite 8, see the proptest below).

use std::io::{BufReader, Read};
use std::path::Path;

use protocol::models::ModelShape;

const MAGIC: &[u8; 4] = b"GGUF";
/// Header metadata is a few MB at most (tokenizer vocabularies); refuse to read more.
pub const MAX_HEADER_BYTES: u64 = 64 * 1024 * 1024;
const MAX_STRING: u64 = 1024 * 1024;
const MAX_KV: u64 = 100_000;
const MAX_ARRAY: u64 = 10_000_000;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GgufError {
    #[error("not a GGUF file")]
    NotGguf,
    #[error("unsupported GGUF version {0}")]
    Version(u32),
    #[error("the file ends inside its header")]
    Truncated,
    #[error("the header is malformed: {0}")]
    Malformed(&'static str),
    #[error("the header doesn't say how big the model is")]
    MissingFields,
}

/// What the header says about the model.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub architecture: String,
    pub shape: ModelShape,
    pub embedding_length: u32,
    pub head_count: u32,
}

struct Reader<R> {
    r: R,
    read: u64,
}

impl<R: Read> Reader<R> {
    fn bytes(&mut self, n: u64) -> Result<Vec<u8>, GgufError> {
        if self.read + n > MAX_HEADER_BYTES {
            return Err(GgufError::Malformed("header too large"));
        }
        let mut buf = vec![0u8; n as usize];
        self.r.read_exact(&mut buf).map_err(|_| GgufError::Truncated)?;
        self.read += n;
        Ok(buf)
    }

    fn skip(&mut self, n: u64) -> Result<(), GgufError> {
        if self.read + n > MAX_HEADER_BYTES {
            return Err(GgufError::Malformed("header too large"));
        }
        let copied = std::io::copy(&mut (&mut self.r).take(n), &mut std::io::sink()).map_err(|_| GgufError::Truncated)?;
        if copied != n {
            return Err(GgufError::Truncated);
        }
        self.read += n;
        Ok(())
    }

    fn u32(&mut self) -> Result<u32, GgufError> {
        Ok(u32::from_le_bytes(self.bytes(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64, GgufError> {
        Ok(u64::from_le_bytes(self.bytes(8)?.try_into().unwrap()))
    }

    fn string(&mut self) -> Result<String, GgufError> {
        let n = self.u64()?;
        if n > MAX_STRING {
            return Err(GgufError::Malformed("string too long"));
        }
        String::from_utf8(self.bytes(n)?).map_err(|_| GgufError::Malformed("string is not UTF-8"))
    }
}

/// A metadata value we care about: integers (to read counts) or a string (the architecture).
enum Value {
    Int(u64),
    Str(String),
    Other,
}

fn fixed_size(ty: u32) -> Option<u64> {
    match ty {
        0 | 1 | 7 => Some(1), // u8, i8, bool
        2 | 3 => Some(2),     // u16, i16
        4..=6 => Some(4),     // u32, i32, f32
        10..=12 => Some(8),   // u64, i64, f64
        _ => None,
    }
}

fn value<R: Read>(r: &mut Reader<R>, ty: u32, depth: u32) -> Result<Value, GgufError> {
    Ok(match ty {
        0 | 7 => Value::Int(r.bytes(1)?[0] as u64),
        1 => Value::Int(r.bytes(1)?[0] as i8 as i64 as u64),
        2 => Value::Int(u16::from_le_bytes(r.bytes(2)?.try_into().unwrap()) as u64),
        3 => Value::Int(i16::from_le_bytes(r.bytes(2)?.try_into().unwrap()) as i64 as u64),
        4 => Value::Int(r.u32()? as u64),
        5 => Value::Int(r.u32()? as i32 as i64 as u64),
        10 | 11 => Value::Int(r.u64()?),
        6 => {
            r.skip(4)?;
            Value::Other
        }
        12 => {
            r.skip(8)?;
            Value::Other
        }
        8 => Value::Str(r.string()?),
        9 => {
            if depth > 2 {
                return Err(GgufError::Malformed("arrays nested too deep"));
            }
            let inner = r.u32()?;
            let n = r.u64()?;
            if n > MAX_ARRAY {
                return Err(GgufError::Malformed("array too long"));
            }
            match fixed_size(inner) {
                Some(size) => r.skip(n.checked_mul(size).ok_or(GgufError::Malformed("array too long"))?)?,
                None => {
                    for _ in 0..n {
                        value(r, inner, depth + 1)?;
                    }
                }
            }
            Value::Other
        }
        _ => return Err(GgufError::Malformed("unknown value type")),
    })
}

/// Parses the header from any reader (the start of a GGUF file).
pub fn read_header(r: impl Read) -> Result<Header, GgufError> {
    let mut r = Reader { r, read: 0 };
    if r.bytes(4).map_err(|_| GgufError::NotGguf)? != MAGIC {
        return Err(GgufError::NotGguf);
    }
    let version = r.u32()?;
    if !(2..=3).contains(&version) {
        return Err(GgufError::Version(version));
    }
    let _tensors = r.u64()?;
    let kv = r.u64()?;
    if kv > MAX_KV {
        return Err(GgufError::Malformed("too many metadata entries"));
    }
    let mut arch = None;
    let mut ints: std::collections::HashMap<String, u64> = Default::default();
    for _ in 0..kv {
        let key = r.string()?;
        let ty = r.u32()?;
        match value(&mut r, ty, 0)? {
            Value::Str(s) if key == "general.architecture" => arch = Some(s),
            Value::Int(n) => {
                ints.insert(key, n);
            }
            _ => {}
        }
    }
    let arch = arch.ok_or(GgufError::MissingFields)?;
    let get = |k: &str| ints.get(&format!("{arch}.{k}")).copied();
    let layers = get("block_count").ok_or(GgufError::MissingFields)?;
    let embd = get("embedding_length").ok_or(GgufError::MissingFields)?;
    let heads = get("attention.head_count").ok_or(GgufError::MissingFields)?;
    let kv_heads = get("attention.head_count_kv").unwrap_or(heads);
    if heads == 0 || layers == 0 || embd == 0 {
        return Err(GgufError::Malformed("zero-sized model"));
    }
    let head_dim = get("attention.key_length").unwrap_or(embd / heads);
    let small = |v: u64| u32::try_from(v).map_err(|_| GgufError::Malformed("implausible size"));
    Ok(Header {
        architecture: arch,
        shape: ModelShape { layers: small(layers)?, kv_heads: small(kv_heads)?, head_dim: small(head_dim)? },
        embedding_length: small(embd)?,
        head_count: small(heads)?,
    })
}

pub fn read_file(path: &Path) -> Result<Header, GgufError> {
    let f = std::fs::File::open(path).map_err(|_| GgufError::Truncated)?;
    read_header(BufReader::new(f))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Builds a GGUF header for tests.
    pub fn build(kvs: &[(&str, u32, Vec<u8>)]) -> Vec<u8> {
        let mut b = b"GGUF".to_vec();
        b.extend(3u32.to_le_bytes());
        b.extend(0u64.to_le_bytes());
        b.extend((kvs.len() as u64).to_le_bytes());
        for (k, ty, v) in kvs {
            b.extend((k.len() as u64).to_le_bytes());
            b.extend(k.as_bytes());
            b.extend(ty.to_le_bytes());
            b.extend(v);
        }
        b
    }

    pub fn s(v: &str) -> Vec<u8> {
        let mut b = (v.len() as u64).to_le_bytes().to_vec();
        b.extend(v.as_bytes());
        b
    }

    pub fn u(v: u32) -> Vec<u8> {
        v.to_le_bytes().to_vec()
    }

    /// A Qwen3-8B-shaped header with a tokenizer array to skip over.
    pub fn qwen_like() -> Vec<u8> {
        let mut tokens = 9u32.to_le_bytes().to_vec(); // array of strings
        tokens.extend(8u32.to_le_bytes());
        tokens.extend(3u64.to_le_bytes());
        for t in ["a", "bb", "ccc"] {
            tokens.extend(s(t));
        }
        build(&[
            ("general.architecture", 8, s("qwen3")),
            ("general.name", 8, s("Qwen3 8B")),
            ("tokenizer.ggml.tokens", 9, tokens[4..].to_vec()),
            ("qwen3.block_count", 4, u(36)),
            ("qwen3.embedding_length", 4, u(4096)),
            ("qwen3.attention.head_count", 4, u(32)),
            ("qwen3.attention.head_count_kv", 4, u(8)),
            ("qwen3.attention.key_length", 4, u(128)),
            ("qwen3.rope.freq_base", 6, 1_000_000f32.to_le_bytes().to_vec()),
        ])
    }

    #[test]
    fn reads_the_shape_and_skips_what_it_doesnt_need() {
        let h = read_header(&qwen_like()[..]).unwrap();
        assert_eq!(h.architecture, "qwen3");
        assert_eq!(h.shape, ModelShape { layers: 36, kv_heads: 8, head_dim: 128 });
    }

    #[test]
    fn head_size_falls_back_to_embedding_over_heads() {
        let b = build(&[
            ("general.architecture", 8, s("llama")),
            ("llama.block_count", 4, u(30)),
            ("llama.embedding_length", 4, u(576)),
            ("llama.attention.head_count", 4, u(9)),
            ("llama.attention.head_count_kv", 4, u(3)),
        ]);
        assert_eq!(read_header(&b[..]).unwrap().shape, ModelShape { layers: 30, kv_heads: 3, head_dim: 64 });
    }

    #[test]
    fn rejects_what_isnt_a_model() {
        assert_eq!(read_header(&b"PK\x03\x04hello"[..]), Err(GgufError::NotGguf));
        assert_eq!(read_header(&b""[..]), Err(GgufError::NotGguf));
        let mut v1 = qwen_like();
        v1[4] = 1;
        assert_eq!(read_header(&v1[..]), Err(GgufError::Version(1)));
        assert_eq!(read_header(&qwen_like()[..40]), Err(GgufError::Truncated));
        let no_arch = build(&[("qwen3.block_count", 4, u(36))]);
        assert_eq!(read_header(&no_arch[..]), Err(GgufError::MissingFields));
    }

    #[test]
    fn absurd_lengths_are_refused_before_allocating() {
        let mut b = b"GGUF".to_vec();
        b.extend(3u32.to_le_bytes());
        b.extend(0u64.to_le_bytes());
        b.extend(1u64.to_le_bytes());
        b.extend(u64::MAX.to_le_bytes()); // key length
        assert!(matches!(read_header(&b[..]), Err(GgufError::Malformed(_))));
    }

    proptest! {
        /// Roadmap suite 8: arbitrary bytes, or a real header with random corruption, give an
        /// error or a result, never a panic.
        #[test]
        fn never_panics_on_garbage(data in proptest::collection::vec(any::<u8>(), 0..2048)) {
            let _ = read_header(&data[..]);
        }

        #[test]
        fn never_panics_on_a_corrupted_header(flips in proptest::collection::vec((0usize..400, any::<u8>()), 1..8)) {
            let mut b = qwen_like();
            for (i, v) in flips {
                let i = i % b.len();
                b[i] = v;
            }
            let _ = read_header(&b[..]);
        }
    }

    #[test]
    #[ignore = "needs the dev model from scripts/dev-engine.sh"]
    fn reads_the_real_dev_model() {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.dev/models/SmolLM2-135M-Instruct-Q8_0.gguf");
        let h = read_file(&p).unwrap();
        assert_eq!(h.architecture, "llama");
        assert_eq!(h.shape, ModelShape { layers: 30, kv_heads: 3, head_dim: 64 });
    }
}
