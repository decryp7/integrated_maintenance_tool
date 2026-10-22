//! The wire types of the parser library (ADR 0026; camelCase JSON, generated into
//! `app/src/app/core/generated/parsers.ts` and re-exported by `api.types.ts`).

use imt_core::parser::library::{Origin, Relation};
use imt_core::parser::ParserError;
use serde::{Deserialize, Serialize};

/// Where a parser of the library comes from.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParserOrigin {
    /// Shipped with IMT (read-only; can be disabled).
    Builtin,
    /// A parser file of the user's.
    User,
    /// A user file replacing the built-in parser with the same id.
    Override,
}

impl From<Origin> for ParserOrigin {
    fn from(o: Origin) -> Self {
        match o {
            Origin::Builtin => ParserOrigin::Builtin,
            Origin::User => ParserOrigin::User,
            Origin::Override => ParserOrigin::Override,
        }
    }
}

/// Why a parser file was refused, with its 1-based line and column when known.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserErrorDto {
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

impl From<&ParserError> for ParserErrorDto {
    fn from(e: &ParserError) -> Self {
        ParserErrorDto {
            message: e.message.clone(),
            line: e.line,
            column: e.column,
        }
    }
}

/// A source of the current window read with a parser.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserSourceDto {
    pub id: u32,
    pub name: String,
}

/// One parser of the library.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserEntryDto {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub category: String,
    /// `[format] kind`: auto, text, json, csv or an importer id.
    pub kind: String,
    pub priority: i32,
    pub origin: ParserOrigin,
    pub enabled: bool,
    /// Content hash of its canonical TOML (what sources record).
    pub hash: String,
    /// The sources of the current window it read.
    pub sources: Vec<ParserSourceDto>,
}

/// A user parser file that does not parse.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokenParserDto {
    pub file: String,
    pub error: ParserErrorDto,
}

/// The library as Settings ▸ Parsers shows it.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserLibraryDto {
    pub parsers: Vec<ParserEntryDto>,
    pub broken: Vec<BrokenParserDto>,
    /// Sources of this window read with a parser that has since changed, been disabled or removed
    /// (`parsers_reread` reads them again).
    pub stale: usize,
}

/// A parser's text.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserDocDto {
    pub id: String,
    pub origin: ParserOrigin,
    pub toml: String,
}

/// Whether a parser text is valid (the editor's check).
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserValidationDto {
    pub ok: bool,
    pub error: Option<ParserErrorDto>,
    pub id: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
}

/// How an imported parser relates to the library.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ParserRelation {
    New,
    Newer,
    Same,
    Older,
}

impl From<Relation> for ParserRelation {
    fn from(r: Relation) -> Self {
        match r {
            Relation::New => ParserRelation::New,
            Relation::Newer => ParserRelation::Newer,
            Relation::Same => ParserRelation::Same,
            Relation::Older => ParserRelation::Older,
        }
    }
}

/// The library's parser with the id of an imported one.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserExistingDto {
    pub origin: ParserOrigin,
    pub version: String,
    pub toml: String,
}

/// What importing a parser file would do (nothing is saved yet).
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserImportCheckDto {
    pub toml: String,
    pub id: String,
    pub name: String,
    pub version: String,
    pub relation: ParserRelation,
    pub existing: Option<ParserExistingDto>,
}

/// What the format wizard asks the engine to compose (`parser_draft`).
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParserDraftRequest {
    /// The file the wizard was opened for (names a new parser).
    pub file_name: String,
    /// The parser to change (the source's); `None` for a new parser.
    pub base: Option<String>,
    pub regex: Option<String>,
    pub template: Option<String>,
    pub timestamp_format: Option<String>,
}

impl From<ParserDraftRequest> for imt_core::parser::draft::DraftRequest {
    fn from(r: ParserDraftRequest) -> Self {
        imt_core::parser::draft::DraftRequest {
            file_name: r.file_name,
            base: r.base,
            regex: r.regex,
            template: r.template,
            timestamp_format: r.timestamp_format,
        }
    }
}

/// The TypeScript declarations of these types.
#[cfg(test)]
pub fn generated(header: &str) -> String {
    use ts_rs::TS;
    let decls = [
        ParserOrigin::decl(),
        ParserErrorDto::decl(),
        ParserSourceDto::decl(),
        ParserEntryDto::decl(),
        BrokenParserDto::decl(),
        ParserLibraryDto::decl(),
        ParserDocDto::decl(),
        ParserValidationDto::decl(),
        ParserRelation::decl(),
        ParserExistingDto::decl(),
        ParserImportCheckDto::decl(),
        ParserDraftRequest::decl(),
    ];
    format!(
        "{header}{}",
        decls.iter().map(|d| format!("export {d}\n")).collect::<Vec<_>>().join("\n")
    )
}
