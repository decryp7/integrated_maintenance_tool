//! Helpers for tests (here and in `tests/`): synthetic GGUF files.

pub mod gguf {
    /// Builds a small GGUF header for tests; `pad` is appended so the file looks big enough.
    pub fn build(arch: &str, extra: &[(&str, Kv)], pad: usize) -> Vec<u8> {
        let mut kvs: Vec<(String, Kv)> = vec![("general.architecture".into(), Kv::S(arch.into()))];
        kvs.extend(extra.iter().map(|(k, v)| (k.to_string(), v.clone())));
        let mut b = b"GGUF".to_vec();
        b.extend(3u32.to_le_bytes());
        b.extend(7u64.to_le_bytes());
        b.extend((kvs.len() as u64).to_le_bytes());
        for (k, v) in kvs {
            b.extend((k.len() as u64).to_le_bytes());
            b.extend(k.as_bytes());
            match v {
                Kv::S(s) => {
                    b.extend(8u32.to_le_bytes());
                    b.extend((s.len() as u64).to_le_bytes());
                    b.extend(s.as_bytes());
                }
                Kv::U32(n) => {
                    b.extend(4u32.to_le_bytes());
                    b.extend(n.to_le_bytes());
                }
                Kv::U64(n) => {
                    b.extend(10u32.to_le_bytes());
                    b.extend(n.to_le_bytes());
                }
                Kv::StrArr(items) => {
                    b.extend(9u32.to_le_bytes());
                    b.extend(8u32.to_le_bytes());
                    b.extend((items.len() as u64).to_le_bytes());
                    for s in items {
                        b.extend((s.len() as u64).to_le_bytes());
                        b.extend(s.as_bytes());
                    }
                }
                Kv::I32Arr(n) => {
                    b.extend(9u32.to_le_bytes());
                    b.extend(5u32.to_le_bytes());
                    b.extend((n as u64).to_le_bytes());
                    b.extend(vec![0u8; n * 4]);
                }
            }
        }
        b.extend(vec![0u8; pad]);
        b
    }

    #[derive(Clone)]
    pub enum Kv {
        S(String),
        U32(u32),
        U64(u64),
        StrArr(Vec<String>),
        I32Arr(usize),
    }

    pub fn qwen_like() -> Vec<u8> {
        build(
            "qwen3",
            &[
                ("general.name", Kv::S("Qwen3 Test".into())),
                ("general.size_label", Kv::S("4B".into())),
                ("general.file_type", Kv::U32(15)),
                ("qwen3.context_length", Kv::U64(32768)),
                ("qwen3.block_count", Kv::U32(36)),
                ("qwen3.embedding_length", Kv::U32(2560)),
                ("qwen3.attention.head_count", Kv::U32(32)),
                ("qwen3.attention.head_count_kv", Kv::U32(8)),
                ("tokenizer.ggml.tokens", Kv::StrArr(vec!["a".into(), "bb".into()])),
                ("tokenizer.ggml.token_type", Kv::I32Arr(10)),
            ],
            1 << 17,
        )
    }
}
