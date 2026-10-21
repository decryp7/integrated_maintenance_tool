// Generated from the Rust wire types by `cargo test -p imt-app --lib generated_types` (ts-rs; ADR 0016).
// Do not edit: change the Rust type and run the test with IMT_WRITE_TYPES=1.

export type ParserOrigin = "builtin" | "user" | "override";

export type ParserErrorDto = { message: string, line: number | null, column: number | null, };

export type ParserSourceDto = { id: number, name: string, };

export type ParserEntryDto = { id: string, name: string, version: string, description: string, category: string, 
/**
 * `[format] kind`: auto, text, json, csv or an importer id.
 */
kind: string, priority: number, origin: ParserOrigin, enabled: boolean, 
/**
 * Content hash of its canonical TOML (what sources record).
 */
hash: string, 
/**
 * The sources of the current window it read.
 */
sources: Array<ParserSourceDto>, };

export type BrokenParserDto = { file: string, error: ParserErrorDto, };

export type ParserLibraryDto = { parsers: Array<ParserEntryDto>, broken: Array<BrokenParserDto>, 
/**
 * Sources of this window read with a parser that has since changed, been disabled or removed
 * (`parsers_reread` reads them again).
 */
stale: number, };

export type ParserDocDto = { id: string, origin: ParserOrigin, toml: string, };

export type ParserValidationDto = { ok: boolean, error: ParserErrorDto | null, id: string | null, name: string | null, version: string | null, };

export type ParserRelation = "new" | "newer" | "same" | "older";

export type ParserExistingDto = { origin: ParserOrigin, version: string, toml: string, };

export type ParserImportCheckDto = { toml: string, id: string, name: string, version: string, relation: ParserRelation, existing: ParserExistingDto | null, };

export type ParserDraftRequest = { 
/**
 * The file the wizard was opened for (names a new parser).
 */
fileName: string, 
/**
 * The parser to change (the source's); `None` for a new parser.
 */
base: string | null, regex: string | null, template: string | null, timestampFormat: string | null, };
