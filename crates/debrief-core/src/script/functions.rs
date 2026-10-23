//! Functions Debrief adds to scripts: `drop()` and regular expressions.

use super::limits::MAX_STRING_SIZE;
use regex::Regex;
use rhai::{Dynamic, Engine, EvalAltResult, Position, INT};
use std::cell::RefCell;
use std::collections::HashMap;

/// Token carried by the error `drop()` raises to end the script and drop the record.
/// It is a termination, which `try`/`catch` in a script cannot intercept.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DropRecord;

/// Compiled patterns kept per import thread; cleared when this many are cached.
const MAX_CACHED_REGEXES: usize = 64;

thread_local! {
    static REGEXES: RefCell<HashMap<String, Result<Regex, String>>> = RefCell::new(HashMap::new());
}

const SLEEP: &str = "sleep() is not available in pipeline scripts";

type FnResult<T> = Result<T, Box<EvalAltResult>>;

fn runtime_error<T>(message: String) -> FnResult<T> {
    Err(Box::new(EvalAltResult::ErrorRuntime(message.into(), Position::NONE)))
}

/// Runs `f` with the compiled `pattern` (bounded by [`crate::user_regex::compile`]).
fn with_regex<T>(pattern: &str, f: impl FnOnce(&Regex) -> FnResult<T>) -> FnResult<T> {
    let compiled = REGEXES.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(hit) = cache.get(pattern) {
            return hit.clone();
        }
        if cache.len() >= MAX_CACHED_REGEXES {
            cache.clear();
        }
        let compiled = crate::user_regex::compile(pattern, false).map_err(|e| format!("invalid regex: {e:#}"));
        cache.insert(pattern.to_string(), compiled.clone());
        compiled
    });
    match compiled {
        Ok(re) => f(&re),
        Err(e) => runtime_error(e),
    }
}

/// Replaces every match of `re` in `text` (`$1`, `${name}` expand groups), failing as soon as
/// the result outgrows the script string limit instead of building it first.
fn replace_all(re: &Regex, text: &str, with: &str) -> FnResult<String> {
    let mut out = String::new();
    let mut last = 0;
    for caps in re.captures_iter(text) {
        let Some(m) = caps.get(0) else { continue };
        out.push_str(&text[last..m.start()]);
        caps.expand(with, &mut out);
        last = m.end();
        if out.len() > MAX_STRING_SIZE {
            return runtime_error(format!("regex_replace result is longer than {MAX_STRING_SIZE} bytes"));
        }
    }
    out.push_str(&text[last..]);
    Ok(out)
}

/// Group `group` of the first match, or `()` without a match or such a group.
fn capture(re: &Regex, text: &str, group: INT) -> Dynamic {
    let group = usize::try_from(group).ok();
    re.captures(text)
        .and_then(|c| group.and_then(|g| c.get(g)))
        .map_or(Dynamic::UNIT, |m| Dynamic::from(m.as_str().to_string()))
}

/// Registers Debrief's functions on a script engine.
pub(crate) fn register(engine: &mut Engine) {
    // The built-in `sleep` would block an import thread without counting operations; functions
    // registered on the engine take precedence over its packages.
    engine.register_fn("sleep", |_: INT| runtime_error::<()>(SLEEP.into()));
    engine.register_fn("sleep", |_: rhai::FLOAT| runtime_error::<()>(SLEEP.into()));
    engine.register_fn("drop", || -> FnResult<()> {
        Err(Box::new(EvalAltResult::ErrorTerminated(Dynamic::from(DropRecord), Position::NONE)))
    });
    engine.register_fn("regex_match", |text: &str, pattern: &str| {
        with_regex(pattern, |re| Ok(re.is_match(text)))
    });
    engine.register_fn("regex_replace", |text: &str, pattern: &str, with: &str| {
        with_regex(pattern, |re| replace_all(re, text, with))
    });
    engine.register_fn("regex_capture", |text: &str, pattern: &str, group: INT| {
        with_regex(pattern, |re| Ok(capture(re, text, group)))
    });
}
