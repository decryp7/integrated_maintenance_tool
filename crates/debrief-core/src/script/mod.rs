//! The `script` pipeline step: a small sandboxed script (Rhai) run once per record, for
//! transforms that the declarative steps cannot express.
//!
//! The script sees one record at a time through these variables:
//! `message` (string), `level` (string, e.g. "Warning"), `timestamp` (ms since the epoch,
//! or `()` when the record has none), `fields` (map of the record's fields with JSON-like
//! values) and `source` (the file name, read-only). It may change `message`, `level`,
//! `timestamp` and `fields` (add, change or remove entries), or call `drop()` to drop the
//! record; otherwise the record is kept.
//!
//! Scripts are compiled once, when the profile loads; syntax errors are profile errors
//! with line and column. The sandbox has no file, network, process, module or `eval`
//! access, and every record runs under the [`limits`]. A runtime error or an exceeded limit
//! leaves that record unchanged and is counted by the caller; it never aborts an import.

mod functions;
pub mod limits;
#[cfg(test)]
mod tests;
mod value;

use crate::model::{Level, Record};
use crate::plugin::Verdict;
use rhai::{Dynamic, Engine, EvalAltResult, OptimizationLevel, Scope, AST};

/// Names of the variables a script sees, in scope order.
const MESSAGE: &str = "message";
const LEVEL: &str = "level";
const TIMESTAMP: &str = "timestamp";
const FIELDS: &str = "fields";
const SOURCE: &str = "source";

/// Built-in keywords removed from the language: `eval` runs text as code. (`sleep` is
/// replaced by a function that fails, see [`functions::register`].)
const DISABLED_SYMBOLS: [&str; 1] = ["eval"];

/// Why a script could not be compiled or run on a record.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScriptError {
    #[error("is empty")]
    Empty,
    #[error("is too long ({0} bytes; at most {max})", max = limits::MAX_SCRIPT_LEN)]
    TooLong(usize),
    /// `line` and `column` count from 1.
    #[error("syntax error at line {line}, column {column}: {message}")]
    Syntax { line: usize, column: usize, message: String },
    /// A runtime error or an exceeded limit, with Rhai's description (and position).
    #[error("{0}")]
    Runtime(String),
}

/// A compiled script with the engine (functions and limits) that runs it. Shared by the
/// threads importing files; each run uses its own [`Runner`].
pub struct Script {
    engine: Engine,
    ast: AST,
}

impl std::fmt::Debug for Script {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Script").finish_non_exhaustive()
    }
}

/// Declares the variables of record `r` in `scope`, in the order [`Script::compile`]
/// declares them and [`Runner::apply`] fills them.
fn push_record(scope: &mut Scope<'static>, r: &Record, source: Dynamic, constant: bool) {
    scope
        .push_dynamic(MESSAGE, Dynamic::from(r.message.clone()))
        .push_dynamic(LEVEL, Dynamic::from(r.level.as_str().to_string()))
        .push_dynamic(TIMESTAMP, r.ts.map_or(Dynamic::UNIT, Dynamic::from_int))
        .push_dynamic(FIELDS, Dynamic::from_map(value::fields_to_map(&r.fields)));
    // At compile time `source` is an ordinary variable so the optimizer cannot fold a
    // placeholder value into the script; at run time it is a constant.
    if constant {
        scope.push_constant_dynamic(SOURCE, source);
    } else {
        scope.push_dynamic(SOURCE, source);
    }
}

fn new_engine() -> Engine {
    // `new_raw`: no module resolver, and print/debug go nowhere.
    let mut engine = Engine::new_raw();
    engine.register_global_module(rhai::packages::Package::as_shared_module(&rhai::packages::StandardPackage::new()));
    for symbol in DISABLED_SYMBOLS {
        engine.disable_symbol(symbol);
    }
    engine
        .set_strict_variables(true)
        .set_allow_shadowing(false)
        .set_optimization_level(OptimizationLevel::Simple);
    limits::apply(&mut engine);
    functions::register(&mut engine);
    engine
}

impl Script {
    /// Parses and checks a script once. Undefined variables are syntax errors.
    pub fn compile(text: &str) -> Result<Script, ScriptError> {
        if text.trim().is_empty() {
            return Err(ScriptError::Empty);
        }
        if text.len() > limits::MAX_SCRIPT_LEN {
            return Err(ScriptError::TooLong(text.len()));
        }
        let engine = new_engine();
        let mut scope = Scope::new();
        push_record(&mut scope, &Record::new(0), Dynamic::from(String::new()), false);
        let ast = engine.compile_with_scope(&scope, text).map_err(|e| ScriptError::Syntax {
            line: e.1.line().unwrap_or(1),
            column: e.1.position().unwrap_or(1),
            message: e.0.to_string(),
        })?;
        Ok(Script { engine, ast })
    }

    /// State for running the script over the records of one file (`source` is its name).
    pub fn runner(&self, source: &str) -> Runner<'_> {
        Runner {
            script: self,
            scope: Scope::new(),
            source: Dynamic::from(source.to_string()),
        }
    }
}

/// Runs a [`Script`] record by record, reusing one scope.
pub struct Runner<'a> {
    script: &'a Script,
    scope: Scope<'static>,
    source: Dynamic,
}

/// What a finished script left in the scope, checked before the record is changed.
struct Output {
    message: String,
    level: Level,
    ts: Option<i64>,
    fields: rhai::Map,
}

impl Runner<'_> {
    /// Runs the script on `r`. On error `r` is unchanged; a panic inside the engine is
    /// contained and reported as an error.
    pub fn apply(&mut self, r: &mut Record) -> Result<Verdict, ScriptError> {
        self.scope.rewind(0);
        push_record(&mut self.scope, r, self.source.clone(), true);
        let Script { engine, ast } = self.script;
        let scope = &mut self.scope;
        let run =
            crate::guard::catch("script", || Ok(engine.run_ast_with_scope(scope, ast))).map_err(|e| ScriptError::Runtime(e.to_string()))?;
        match run {
            Ok(()) => {}
            Err(e) if is_drop(&e) => return Ok(Verdict::Drop),
            Err(e) if matches!(*e, EvalAltResult::Exit(..)) => {}
            Err(e) => return Err(ScriptError::Runtime(e.to_string())),
        }
        let out = self.output()?;
        r.message = out.message;
        r.level = out.level;
        r.ts = out.ts;
        r.fields = value::map_to_fields(&r.fields, out.fields);
        Ok(Verdict::Keep)
    }

    fn take(&mut self, name: &str) -> Dynamic {
        self.scope.get_mut(name).map(std::mem::take).unwrap_or_default().flatten()
    }

    fn output(&mut self) -> Result<Output, ScriptError> {
        let wrong = |name: &str, want: &str, got: &str| ScriptError::Runtime(format!("`{name}` must be {want}, not {got}"));
        let message = self.take(MESSAGE);
        let message = message.into_string().map_err(|t| wrong(MESSAGE, "a string", t))?;
        let level = self.take(LEVEL);
        let level = match level.into_string() {
            Ok(s) => {
                Level::parse(&s).ok_or_else(|| ScriptError::Runtime(format!("`level` \"{s}\" is not a level (use e.g. \"Warning\")")))?
            }
            Err(t) => return Err(wrong(LEVEL, "a string", t)),
        };
        let ts = self.take(TIMESTAMP);
        let ts = if ts.is_unit() {
            None
        } else if let Ok(i) = ts.as_int() {
            Some(i)
        } else {
            match ts.as_float() {
                Ok(f) if f.is_finite() => Some(f.round() as i64),
                _ => return Err(wrong(TIMESTAMP, "milliseconds or ()", ts.type_name())),
            }
        };
        let fields = self.take(FIELDS);
        let got = fields.type_name();
        let fields = fields.try_cast::<rhai::Map>().ok_or_else(|| wrong(FIELDS, "a map", got))?;
        // Rhai checks map sizes when a map is used; `fields` itself is only checked here.
        if fields.len() > limits::MAX_MAP_SIZE {
            return Err(ScriptError::Runtime(format!(
                "`fields` has more than {} entries",
                limits::MAX_MAP_SIZE
            )));
        }
        Ok(Output {
            message,
            level,
            ts,
            fields,
        })
    }
}

/// Whether the error is the termination raised by `drop()` (also from inside a function).
fn is_drop(e: &EvalAltResult) -> bool {
    matches!(e.unwrap_inner(), EvalAltResult::ErrorTerminated(token, _) if token.is::<functions::DropRecord>())
}
