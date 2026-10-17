//! A tiny bounded reader for the header of GGUF model files: name, architecture, quantisation,
//! context length and the numbers needed to estimate memory. It never allocates from untrusted
//! sizes without a limit, never reads past the file and never panics (covered by truncation tests).

use crate::error::{AiError, Result};
use crate::recommend::Quant;
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const MAGIC: &[u8; 4] = b"GGUF";
const MAX_KV: u64 = 8192;
const MAX_KEY_LEN: u64 = 256;
const MAX_STRING_LEN: u64 = 1 << 20;
const MAX_ARRAY_ITEMS: u64 = 8_000_000;
const MIN_FILE: u64 = 1 << 16;

/// Architectures that are not language models.
const NOT_A_LANGUAGE_MODEL: &[&str] = &[
    "clip",
    "mmproj",
    "whisper",
    "stable-diffusion",
    "sam",
    "vae",
    "t5encoder",
    "bert",
    "nomic-bert",
    "jina-bert-v2",
];
const KNOWN: &[&str] = &[
    "llama",
    "qwen2",
    "qwen2moe",
    "qwen3",
    "qwen3moe",
    "qwen35",
    "qwen35moe",
    "gemma",
    "gemma2",
    "gemma3",
    "gemma3n",
    "gemma4",
    "phi2",
    "phi3",
    "mistral3",
    "mixtral",
    "deepseek2",
    "falcon",
    "starcoder2",
    "granite",
    "olmo2",
    "smollm3",
    "glm4",
    "command-r",
    "exaone",
];

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GgufInfo {
    pub version: u32,
    pub architecture: String,
    pub name: Option<String>,
    pub quant: Option<String>,
    pub context_length: Option<u64>,
    pub block_count: Option<u32>,
    pub embedding_length: Option<u32>,
    pub head_count: Option<u32>,
    pub head_count_kv: Option<u32>,
    pub expert_count: Option<u32>,
    pub tensor_count: u64,
    pub file_size: u64,
    /// The Jinja chat template stored in the file (used only to pick the prompt format).
    #[serde(skip)]
    pub chat_template: Option<String>,
    /// Parameters in billions: from `general.size_label` when present, else estimated from the size.
    pub params_b: f64,
    /// The architecture is one this build of llama.cpp is known to run.
    pub known_architecture: bool,
}

impl GgufInfo {
    pub fn is_moe(&self) -> bool {
        self.expert_count.unwrap_or(0) > 1
    }

    /// Bytes of KV cache per token (f16 keys and values).
    pub fn kv_bytes_per_token(&self) -> u64 {
        match (self.block_count, self.embedding_length, self.head_count, self.head_count_kv) {
            (Some(l), Some(e), Some(h), kv) if h > 0 => {
                let kv = kv.unwrap_or(h).max(1) as u64;
                2 * l as u64 * kv * (e as u64 / h as u64) * 2
            }
            // Unknown layout: a typical 7-9B model.
            _ => 128 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Val {
    Str(String),
    Int(u64),
}

struct Reader<R> {
    r: R,
    len: u64,
}

fn eof(what: &str) -> AiError {
    AiError::ModelFile(format!("the file is truncated or damaged ({what})"))
}

impl<R: Read + Seek> Reader<R> {
    fn bytes<const N: usize>(&mut self, what: &str) -> Result<[u8; N]> {
        let mut b = [0u8; N];
        self.r.read_exact(&mut b).map_err(|_| eof(what))?;
        Ok(b)
    }
    fn u32(&mut self, w: &str) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes::<4>(w)?))
    }
    fn u64(&mut self, w: &str) -> Result<u64> {
        Ok(u64::from_le_bytes(self.bytes::<8>(w)?))
    }
    fn skip(&mut self, n: u64, w: &str) -> Result<()> {
        let pos = self.r.stream_position().map_err(|_| eof(w))?;
        if pos.checked_add(n).is_none_or(|e| e > self.len) {
            return Err(eof(w));
        }
        self.r.seek(SeekFrom::Current(n as i64)).map_err(|_| eof(w))?;
        Ok(())
    }
    fn string(&mut self, max: u64, w: &str) -> Result<String> {
        let n = self.u64(w)?;
        if n > max {
            return Err(AiError::ModelFile(format!("a text field is unreasonably long ({w})")));
        }
        let mut buf = vec![0u8; n as usize];
        self.r.read_exact(&mut buf).map_err(|_| eof(w))?;
        Ok(String::from_utf8_lossy(&buf).into_owned())
    }
    fn skip_string(&mut self, w: &str) -> Result<()> {
        let n = self.u64(w)?;
        self.skip(n, w)
    }
}

fn scalar_size(t: u32) -> Option<u64> {
    match t {
        0 | 1 | 7 => Some(1),
        2 | 3 => Some(2),
        4..=6 => Some(4),
        10..=12 => Some(8),
        _ => None,
    }
}

fn wanted(key: &str) -> bool {
    matches!(
        key,
        "general.architecture" | "general.name" | "general.size_label" | "general.file_type" | "tokenizer.chat_template"
    ) || [
        ".context_length",
        ".block_count",
        ".embedding_length",
        ".attention.head_count",
        ".attention.head_count_kv",
        ".expert_count",
    ]
    .iter()
    .any(|s| key.ends_with(s))
}

fn read_value<R: Read + Seek>(rd: &mut Reader<R>, t: u32, keep: bool) -> Result<Option<Val>> {
    match t {
        8 => {
            if keep {
                Ok(Some(Val::Str(rd.string(MAX_STRING_LEN, "value")?)))
            } else {
                rd.skip_string("value")?;
                Ok(None)
            }
        }
        9 => {
            let et = rd.u32("array type")?;
            let n = rd.u64("array length")?;
            if n > MAX_ARRAY_ITEMS {
                return Err(AiError::ModelFile("an array in the header is unreasonably long".into()));
            }
            match (et, scalar_size(et)) {
                (_, Some(sz)) => rd.skip(n.checked_mul(sz).ok_or_else(|| eof("array"))?, "array")?,
                (8, None) => {
                    for _ in 0..n {
                        rd.skip_string("array item")?;
                    }
                }
                _ => return Err(AiError::ModelFile("nested arrays in the header are not supported".into())),
            }
            Ok(None)
        }
        _ => {
            let Some(sz) = scalar_size(t) else {
                return Err(AiError::ModelFile(format!("unknown value type {t} in the header")));
            };
            let mut b = [0u8; 8];
            let mut raw = vec![0u8; sz as usize];
            rd.r.read_exact(&mut raw).map_err(|_| eof("value"))?;
            b[..raw.len()].copy_from_slice(&raw);
            Ok(keep.then(|| Val::Int(u64::from_le_bytes(b))))
        }
    }
}

fn quant_of(file_type: u32) -> Option<&'static str> {
    Some(match file_type {
        0 => "F32",
        1 => "F16",
        2 => "Q4_0",
        3 => "Q4_1",
        7 => "Q8_0",
        8 => "Q5_0",
        9 => "Q5_1",
        10 => "Q2_K",
        11 => "Q3_K_S",
        12 => "Q3_K_M",
        13 => "Q3_K_L",
        14 => "Q4_K_S",
        15 => "Q4_K_M",
        16 => "Q5_K_S",
        17 => "Q5_K_M",
        18 => "Q6_K",
        19..=31 => "IQ",
        32 => "BF16",
        _ => return None,
    })
}

/// "9B" -> 9.0, "35B-A3B" -> 35.0, "270M" -> 0.27.
pub fn parse_size_label(s: &str) -> Option<f64> {
    let s = s.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let n: f64 = s[..split].parse().ok()?;
    match s[split..].chars().next()?.to_ascii_uppercase() {
        'B' => Some(n),
        'M' => Some(n / 1000.0),
        'T' => Some(n * 1000.0),
        _ => None,
    }
    .filter(|v| v.is_finite() && *v > 0.0)
}

/// Reads the header of `r` (a stream of `len` bytes).
pub fn parse<R: Read + Seek>(r: R, len: u64) -> Result<GgufInfo> {
    let mut rd = Reader { r, len };
    let mut magic = [0u8; 4];
    rd.r.read_exact(&mut magic)
        .map_err(|_| AiError::ModelFile("the file is empty or unreadable".into()))?;
    if &magic != MAGIC {
        return Err(AiError::ModelFile(if magic == *b"<!DO" || magic == *b"<htm" || magic == *b"<HTM" {
            "this is a web page, not a model (the download probably failed)".to_string()
        } else {
            "this is not a GGUF file".to_string()
        }));
    }
    let version = rd.u32("version")?;
    if !(2..=3).contains(&version) {
        return Err(AiError::ModelFile(format!("GGUF version {version} is not supported")));
    }
    let tensor_count = rd.u64("tensor count")?;
    let kv_count = rd.u64("entry count")?;
    if kv_count > MAX_KV || tensor_count > 4_000_000 {
        return Err(AiError::ModelFile("the header claims an unreasonable number of entries".into()));
    }
    let mut vals: HashMap<String, Val> = HashMap::new();
    for _ in 0..kv_count {
        let key = rd.string(MAX_KEY_LEN, "key")?;
        let t = rd.u32("value type")?;
        if let Some(v) = read_value(&mut rd, t, wanted(&key))? {
            vals.insert(key, v);
        }
    }
    let text = |k: &str| match vals.get(k) {
        Some(Val::Str(s)) => Some(s.clone()),
        _ => None,
    };
    let int = |k: &str| match vals.get(k) {
        Some(Val::Int(n)) => Some(*n),
        _ => None,
    };
    let architecture = text("general.architecture").ok_or_else(|| AiError::ModelFile("the header has no architecture".into()))?;
    if NOT_A_LANGUAGE_MODEL.contains(&architecture.as_str()) {
        return Err(AiError::ModelFile(format!(
            "this is a '{architecture}' file, not a language model (for vision models choose the main model file, not the mmproj file)"
        )));
    }
    let per_arch = |s: &str| int(&format!("{architecture}.{s}"));
    let quant = int("general.file_type")
        .and_then(|n| u32::try_from(n).ok())
        .and_then(quant_of)
        .map(str::to_string);
    let params_b = text("general.size_label").and_then(|s| parse_size_label(&s)).unwrap_or_else(|| {
        let q = quant.as_deref().and_then(Quant::parse).unwrap_or(Quant::Q4);
        (len as f64 / 1e9) / q.gb_per_b()
    });
    if len < MIN_FILE {
        return Err(eof("the file is far too small to hold a model"));
    }
    Ok(GgufInfo {
        version,
        known_architecture: KNOWN.contains(&architecture.as_str()),
        name: text("general.name"),
        quant,
        context_length: per_arch("context_length"),
        block_count: per_arch("block_count").map(|n| n as u32),
        embedding_length: per_arch("embedding_length").map(|n| n as u32),
        head_count: per_arch("attention.head_count").map(|n| n as u32),
        head_count_kv: per_arch("attention.head_count_kv").map(|n| n as u32),
        expert_count: per_arch("expert_count").map(|n| n as u32),
        chat_template: text("tokenizer.chat_template"),
        architecture,
        tensor_count,
        file_size: len,
        params_b,
    })
}

/// Validates and describes a model file on disk.
pub fn inspect(path: &Path) -> Result<GgufInfo> {
    let meta = std::fs::metadata(path).map_err(|e| AiError::ModelFile(format!("cannot read {}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(AiError::ModelFile(format!("{} is not a file", path.display())));
    }
    let f = std::fs::File::open(path).map_err(|e| AiError::ModelFile(format!("cannot open {}: {e}", path.display())))?;
    parse(std::io::BufReader::new(f), meta.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::gguf::*;
    use std::io::Cursor;

    fn info(b: &[u8]) -> Result<GgufInfo> {
        parse(Cursor::new(b.to_vec()), b.len() as u64)
    }

    #[test]
    fn reads_the_interesting_fields() {
        let i = info(&qwen_like()).unwrap();
        assert_eq!(i.architecture, "qwen3");
        assert_eq!(i.name.as_deref(), Some("Qwen3 Test"));
        assert_eq!(i.quant.as_deref(), Some("Q4_K_M"));
        assert_eq!(i.context_length, Some(32768));
        assert_eq!((i.block_count, i.head_count, i.head_count_kv), (Some(36), Some(32), Some(8)));
        assert_eq!(i.params_b, 4.0);
        assert!(i.known_architecture && !i.is_moe());
        // 2 (k,v) * 36 layers * 8 kv heads * head_dim 80 * 2 bytes
        assert_eq!(i.kv_bytes_per_token(), 2 * 36 * 8 * 80 * 2);
    }

    #[test]
    fn estimates_params_and_unknown_layout() {
        let b = build("mystery", &[], 1 << 20);
        let i = info(&b).unwrap();
        assert!(!i.known_architecture);
        assert!(i.params_b > 0.0);
        assert_eq!(i.kv_bytes_per_token(), 128 * 1024);
        let moe = build("qwen35moe", &[("qwen35moe.expert_count", Kv::U32(128))], 1 << 17);
        assert!(info(&moe).unwrap().is_moe());
    }

    #[test]
    fn rejects_non_models_with_clear_messages() {
        for (bytes, needle) in [
            (b"<!DOCTYPE html>".to_vec(), "web page"),
            (b"<html>....".to_vec(), "web page"),
            (b"PK\x03\x04zip".to_vec(), "not a GGUF"),
            (vec![], "empty"),
        ] {
            let e = info(&bytes).unwrap_err().to_string();
            assert!(e.contains(needle), "{e}");
        }
        let proj = build("clip", &[], 1 << 17);
        assert!(info(&proj).unwrap_err().to_string().contains("not a language model"));
        let mut v1 = qwen_like();
        v1[4] = 9;
        assert!(info(&v1).unwrap_err().to_string().contains("version"));
        let tiny = build("llama", &[], 0);
        assert!(info(&tiny).unwrap_err().to_string().contains("too small"));
    }

    #[test]
    fn every_truncation_is_an_error_not_a_panic() {
        let full = qwen_like();
        let header_end = full.len() - (1 << 17);
        for cut in 0..header_end {
            assert!(info(&full[..cut]).is_err(), "cut at {cut}");
        }
    }

    #[test]
    fn hostile_headers_are_bounded() {
        let mut b = b"GGUF".to_vec();
        b.extend(3u32.to_le_bytes());
        b.extend(0u64.to_le_bytes());
        b.extend(u64::MAX.to_le_bytes());
        assert!(info(&b).unwrap_err().to_string().contains("unreasonable"));
        // a key longer than allowed
        let mut b = b"GGUF".to_vec();
        b.extend(3u32.to_le_bytes());
        b.extend(0u64.to_le_bytes());
        b.extend(1u64.to_le_bytes());
        b.extend(u64::MAX.to_le_bytes());
        assert!(info(&b).is_err());
        // unknown value type, huge array, nested array, array that runs past the end
        for tail in [
            [20u32.to_le_bytes().to_vec()].concat(),
            [
                9u32.to_le_bytes().to_vec(),
                4u32.to_le_bytes().to_vec(),
                u64::MAX.to_le_bytes().to_vec(),
            ]
            .concat(),
            [
                9u32.to_le_bytes().to_vec(),
                9u32.to_le_bytes().to_vec(),
                1u64.to_le_bytes().to_vec(),
            ]
            .concat(),
            [
                9u32.to_le_bytes().to_vec(),
                4u32.to_le_bytes().to_vec(),
                1_000_000u64.to_le_bytes().to_vec(),
            ]
            .concat(),
        ] {
            let mut b = b"GGUF".to_vec();
            b.extend(3u32.to_le_bytes());
            b.extend(0u64.to_le_bytes());
            b.extend(1u64.to_le_bytes());
            b.extend(1u64.to_le_bytes());
            b.push(b'k');
            b.extend(tail);
            assert!(info(&b).is_err());
        }
        // a wanted string value with a huge length
        let mut b = b"GGUF".to_vec();
        b.extend(3u32.to_le_bytes());
        b.extend(0u64.to_le_bytes());
        b.extend(1u64.to_le_bytes());
        let key = b"general.name";
        b.extend((key.len() as u64).to_le_bytes());
        b.extend(key);
        b.extend(8u32.to_le_bytes());
        b.extend((MAX_STRING_LEN + 1).to_le_bytes());
        assert!(info(&b).unwrap_err().to_string().contains("unreasonably"));
        // no architecture
        let mut b = b"GGUF".to_vec();
        b.extend(3u32.to_le_bytes());
        b.extend(0u64.to_le_bytes());
        b.extend(0u64.to_le_bytes());
        assert!(info(&b).unwrap_err().to_string().contains("architecture"));
    }

    #[test]
    fn inspect_reads_files() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("m.gguf");
        std::fs::write(&p, qwen_like()).unwrap();
        assert_eq!(inspect(&p).unwrap().architecture, "qwen3");
        assert!(inspect(&dir.path().join("none.gguf"))
            .unwrap_err()
            .to_string()
            .contains("cannot read"));
        assert!(inspect(dir.path()).unwrap_err().to_string().contains("not a file"));
    }

    #[test]
    fn size_labels() {
        assert_eq!(parse_size_label("9B"), Some(9.0));
        assert_eq!(parse_size_label("35B-A3B"), Some(35.0));
        assert_eq!(parse_size_label("270M"), Some(0.27));
        assert_eq!(parse_size_label("1T"), Some(1000.0));
        assert_eq!(parse_size_label("x"), None);
        assert_eq!(parse_size_label("5Q"), None);
        assert_eq!(parse_size_label("0B"), None);
    }

    #[test]
    fn quant_names() {
        assert_eq!(quant_of(7), Some("Q8_0"));
        assert_eq!(quant_of(25), Some("IQ"));
        assert_eq!(quant_of(999), None);
    }
}
