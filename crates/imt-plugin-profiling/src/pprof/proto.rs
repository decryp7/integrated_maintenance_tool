//! The pprof wire format: `profile.proto` as prost messages.
//!
//! Vendored generated code. Field numbers and semantics follow
//! `proto/profile.proto` of github.com/google/pprof (Copyright 2016 Google
//! Inc., Apache License 2.0, <https://www.apache.org/licenses/LICENSE-2.0>);
//! the structs are written by hand in the shape `prost-build` generates, so
//! the crate needs no `protoc` or build script. Fields this importer does not
//! read (mapping addresses, build ids, `doc_url`) are kept where they are
//! cheap so encoders (the sample generator, tests) can write complete
//! profiles.
//!
//! String fields of the original (`int64` indexes into `string_table`) stay
//! `i64` here; every index is validated by the importer before use.

/// A profile: samples (one value per [`ValueType`]) over locations, with a
/// shared string table whose entry 0 is the empty string.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Profile {
    #[prost(message, repeated, tag = "1")]
    pub sample_type: Vec<ValueType>,
    #[prost(message, repeated, tag = "2")]
    pub sample: Vec<Sample>,
    #[prost(message, repeated, tag = "3")]
    pub mapping: Vec<Mapping>,
    #[prost(message, repeated, tag = "4")]
    pub location: Vec<Location>,
    #[prost(message, repeated, tag = "5")]
    pub function: Vec<Function>,
    #[prost(string, repeated, tag = "6")]
    pub string_table: Vec<String>,
    #[prost(int64, tag = "7")]
    pub drop_frames: i64,
    #[prost(int64, tag = "8")]
    pub keep_frames: i64,
    #[prost(int64, tag = "9")]
    pub time_nanos: i64,
    #[prost(int64, tag = "10")]
    pub duration_nanos: i64,
    #[prost(message, optional, tag = "11")]
    pub period_type: Option<ValueType>,
    #[prost(int64, tag = "12")]
    pub period: i64,
    #[prost(int64, repeated, tag = "13")]
    pub comment: Vec<i64>,
    #[prost(int64, tag = "14")]
    pub default_sample_type: i64,
}

/// A sample type: `type` and `unit` are string table indexes.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ValueType {
    #[prost(int64, tag = "1")]
    pub r#type: i64,
    #[prost(int64, tag = "2")]
    pub unit: i64,
}

/// One stack (leaf location first) with one value per sample type.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Sample {
    #[prost(uint64, repeated, tag = "1")]
    pub location_id: Vec<u64>,
    #[prost(int64, repeated, tag = "2")]
    pub value: Vec<i64>,
    #[prost(message, repeated, tag = "3")]
    pub label: Vec<Label>,
}

/// A sample label: string (`str`) or numeric (`num`, with `num_unit`).
#[derive(Clone, PartialEq, prost::Message)]
pub struct Label {
    #[prost(int64, tag = "1")]
    pub key: i64,
    #[prost(int64, tag = "2")]
    pub str: i64,
    #[prost(int64, tag = "3")]
    pub num: i64,
    #[prost(int64, tag = "4")]
    pub num_unit: i64,
}

/// A binary or library mapped into the process.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Mapping {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(uint64, tag = "2")]
    pub memory_start: u64,
    #[prost(uint64, tag = "3")]
    pub memory_limit: u64,
    #[prost(uint64, tag = "4")]
    pub file_offset: u64,
    #[prost(int64, tag = "5")]
    pub filename: i64,
    #[prost(int64, tag = "6")]
    pub build_id: i64,
    #[prost(bool, tag = "7")]
    pub has_functions: bool,
    #[prost(bool, tag = "8")]
    pub has_filenames: bool,
    #[prost(bool, tag = "9")]
    pub has_line_numbers: bool,
    #[prost(bool, tag = "10")]
    pub has_inline_frames: bool,
}

/// A code address; `line` lists the inlined functions there, the last entry
/// being the caller the others were inlined into.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Location {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(uint64, tag = "2")]
    pub mapping_id: u64,
    #[prost(uint64, tag = "3")]
    pub address: u64,
    #[prost(message, repeated, tag = "4")]
    pub line: Vec<Line>,
    #[prost(bool, tag = "5")]
    pub is_folded: bool,
}

/// A source line of a function.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Line {
    #[prost(uint64, tag = "1")]
    pub function_id: u64,
    #[prost(int64, tag = "2")]
    pub line: i64,
    #[prost(int64, tag = "3")]
    pub column: i64,
}

/// A function; names and file name are string table indexes.
#[derive(Clone, PartialEq, prost::Message)]
pub struct Function {
    #[prost(uint64, tag = "1")]
    pub id: u64,
    #[prost(int64, tag = "2")]
    pub name: i64,
    #[prost(int64, tag = "3")]
    pub system_name: i64,
    #[prost(int64, tag = "4")]
    pub filename: i64,
    #[prost(int64, tag = "5")]
    pub start_line: i64,
}
