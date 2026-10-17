//! Pure helpers over frame names: splitting a qualified name into namespace,
//! type and method (for group-by), and recognising framework/system code
//! (for "just my code").

use crate::error::{ProfileError, Result};
use crate::limits::query::{MAX_PATTERNS, MAX_PATTERN_LEN};
use crate::model::{Frame, FrameKind};
use regex::{Regex, RegexBuilder};

/// A qualified function name split for grouping. `namespace` and
/// `type_name` are empty when the name has no such part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameParts {
    pub namespace: String,
    pub type_name: String,
    pub method: String,
}

/// Splits `System.Collections.Generic.List<T>.Sort(int)` or
/// `ns::Type<T>::method(int) const` into namespace, type and method.
/// Arguments and generic/template arguments are ignored; names in
/// parentheses (`(garbage collector)`) and addresses are kept whole.
pub fn split_name(function: &str) -> NameParts {
    let whole = || NameParts {
        namespace: String::new(),
        type_name: String::new(),
        method: function.to_string(),
    };
    if function.starts_with('(') || function.starts_with("0x") || function.starts_with('[') {
        return whole();
    }
    let stripped = strip_arguments(function);
    let (sep, parts): (&str, Vec<&str>) = if stripped.contains("::") {
        ("::", stripped.split("::").collect())
    } else {
        (".", stripped.split('.').collect())
    };
    let parts: Vec<&str> = parts.into_iter().filter(|p| !p.is_empty()).collect();
    match parts.len() {
        0 => whole(),
        1 => NameParts {
            namespace: String::new(),
            type_name: String::new(),
            method: parts[0].to_string(),
        },
        n => NameParts {
            namespace: parts[..n - 2].join(sep),
            type_name: parts[n - 2].to_string(),
            method: parts[n - 1].to_string(),
        },
    }
}

/// The name without the argument list and without generic/template arguments.
fn strip_arguments(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut depth = 0usize;
    for c in name.chars() {
        match c {
            '<' | '[' => depth += 1,
            '>' | ']' => depth = depth.saturating_sub(1),
            '(' if depth == 0 => break,
            // `void ns::T::run`: what came before a space was a return type.
            ' ' if depth == 0 && !out.ends_with("operator") => out.clear(),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out.trim().to_string()
}

/// The qualified type (`namespace.Type`) of a function, or `None` for a free function.
pub fn type_key(function: &str) -> Option<String> {
    let p = split_name(function);
    if p.type_name.is_empty() {
        return None;
    }
    let sep = if function.contains("::") { "::" } else { "." };
    Some(if p.namespace.is_empty() {
        p.type_name
    } else {
        format!("{}{sep}{}", p.namespace, p.type_name)
    })
}

/// The namespace of a function, or `None`.
pub fn namespace_key(function: &str) -> Option<String> {
    let p = split_name(function);
    (!p.namespace.is_empty()).then_some(p.namespace)
}

/// Module and function prefixes treated as framework or system code when an
/// importer has nothing better. Matched case-insensitively.
const FRAMEWORK_MODULES: &[&str] = &[
    "system.",
    "microsoft.",
    "mscorlib",
    "netstandard",
    "windowsbase",
    "presentationcore",
    "presentationframework",
    "coreclr",
    "clrjit",
    "libcoreclr",
    "libclrjit",
    "ntdll",
    "kernel32",
    "kernelbase",
    "ucrtbase",
    "msvcrt",
    "vcruntime",
    "libc.so",
    "libc-",
    "libm.so",
    "libpthread",
    "libstdc++",
    "libgcc",
    "ld-linux",
    "libsystem_",
    "libdyld",
    "dyld",
    "[kernel",
    "[vdso]",
    "node:",
];

const FRAMEWORK_FUNCTIONS: &[&str] = &[
    "system.",
    "microsoft.",
    "std::",
    "__gnu_cxx::",
    "node:internal",
    "(program)",
    "(idle)",
];

/// Whether a frame is framework/system code by the built-in rules.
pub fn is_framework(function: &str, module: Option<&str>, kind: FrameKind) -> bool {
    if kind == FrameKind::Kernel {
        return true;
    }
    let module_base = module.map(|m| basename(m).to_ascii_lowercase());
    if let Some(m) = &module_base {
        if FRAMEWORK_MODULES.iter().any(|p| m.starts_with(p)) {
            return true;
        }
    }
    if let Some(m) = module {
        let lower = m.to_ascii_lowercase();
        if lower.starts_with("node:") || lower.starts_with("/usr/lib/") || lower.starts_with("/lib/") || lower.starts_with("/system/") {
            return true;
        }
    }
    let f = function.to_ascii_lowercase();
    FRAMEWORK_FUNCTIONS.iter().any(|p| f.starts_with(p))
}

/// The last path component (`/usr/lib/libc.so.6` → `libc.so.6`, `C:\x\a.dll` → `a.dll`).
pub fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// User patterns for "just my code": `*` matches any text, matched
/// case-insensitively against a frame's module (full path and file name)
/// and its function name.
#[derive(Debug, Clone)]
pub struct FrameworkMatcher {
    patterns: Vec<Regex>,
    /// Also honour each frame's own `is_framework` flag.
    use_flags: bool,
}

impl FrameworkMatcher {
    /// Validates and compiles `patterns` (at most [`MAX_PATTERNS`], each at most [`MAX_PATTERN_LEN`]).
    pub fn new(patterns: &[String], use_flags: bool) -> Result<FrameworkMatcher> {
        if patterns.len() > MAX_PATTERNS {
            return Err(ProfileError::invalid(format!(
                "at most {MAX_PATTERNS} framework patterns are allowed"
            )));
        }
        let mut compiled = Vec::with_capacity(patterns.len());
        for p in patterns {
            let p = p.trim();
            if p.is_empty() {
                continue;
            }
            if p.len() > MAX_PATTERN_LEN {
                return Err(ProfileError::invalid(format!(
                    "framework pattern is longer than {MAX_PATTERN_LEN} characters"
                )));
            }
            let re = format!("^{}$", regex::escape(p).replace(r"\*", ".*").replace(r"\?", "."));
            let re = RegexBuilder::new(&re)
                .case_insensitive(true)
                .size_limit(1 << 20)
                .build()
                .map_err(|e| ProfileError::invalid(format!("framework pattern \"{p}\": {e}")))?;
            compiled.push(re);
        }
        Ok(FrameworkMatcher {
            patterns: compiled,
            use_flags,
        })
    }

    pub fn matches(&self, frame: &Frame) -> bool {
        if self.use_flags && frame.is_framework {
            return true;
        }
        self.patterns
            .iter()
            .any(|re| re.is_match(&frame.function) || frame.module.as_deref().is_some_and(|m| re.is_match(m) || re.is_match(basename(m))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(function: &str, module: Option<&str>, fw: bool) -> Frame {
        Frame {
            function: function.into(),
            module: module.map(Into::into),
            file: None,
            line: None,
            kind: FrameKind::Managed,
            is_framework: fw,
        }
    }

    #[test]
    fn splits_dotnet_cpp_and_plain_names() {
        let p = split_name("System.Collections.Generic.List<T>.Sort(int, string)");
        assert_eq!(p.namespace, "System.Collections.Generic");
        assert_eq!(p.type_name, "List");
        assert_eq!(p.method, "Sort");
        let p = split_name("std::vector<int, std::allocator<int>>::push_back(int const&)");
        assert_eq!(
            (p.namespace.as_str(), p.type_name.as_str(), p.method.as_str()),
            ("std", "vector", "push_back")
        );
        let p = split_name("main");
        assert_eq!((p.namespace.as_str(), p.type_name.as_str(), p.method.as_str()), ("", "", "main"));
        let p = split_name("(garbage collector)");
        assert_eq!(p.method, "(garbage collector)");
        assert_eq!(split_name("0x7ff00012").method, "0x7ff00012");
        assert_eq!(split_name("[unknown]").method, "[unknown]");
        let p = split_name("Shop.Orders.OrderService.Place");
        assert_eq!(
            type_key("Shop.Orders.OrderService.Place").as_deref(),
            Some("Shop.Orders.OrderService")
        );
        assert_eq!(namespace_key("Shop.Orders.OrderService.Place").as_deref(), Some("Shop.Orders"));
        assert_eq!(p.method, "Place");
        assert_eq!(type_key("a::B::c").as_deref(), Some("a::B"));
        assert_eq!(type_key("T.m").as_deref(), Some("T"));
        assert_eq!(type_key("free"), None);
        assert_eq!(namespace_key("T.m"), None);
        // A C++ return type before the name is skipped.
        assert_eq!(split_name("void ns::T::run()").method, "run");
        assert_eq!(split_name("int helper(int)").method, "helper");
        assert_eq!(split_name("...").method, "...");
    }

    #[test]
    fn framework_rules() {
        assert!(is_framework("x", Some("System.Private.CoreLib.dll"), FrameKind::Managed));
        assert!(is_framework("x", Some("/usr/lib/x86_64-linux-gnu/libc.so.6"), FrameKind::Native));
        assert!(is_framework("x", Some(r"C:\Windows\System32\ntdll.dll"), FrameKind::Native));
        assert!(is_framework("schedule", None, FrameKind::Kernel));
        assert!(is_framework("std::sort", None, FrameKind::Native));
        assert!(is_framework("x", Some("node:internal/timers"), FrameKind::Managed));
        assert!(is_framework("x", Some("/lib/foo.so"), FrameKind::Native));
        assert!(!is_framework("Shop.Place", Some("Shop.dll"), FrameKind::Managed));
        assert_eq!(basename("a/b\\c.dll"), "c.dll");
    }

    #[test]
    fn matcher_patterns_and_flags() {
        let m = FrameworkMatcher::new(&["Newtonsoft.*".into(), " ".into(), "*.Internal.?un".into()], true).unwrap();
        assert!(m.matches(&frame("Newtonsoft.Json.JsonConvert.Serialize", None, false)));
        assert!(
            m.matches(&frame("x", Some("/opt/newtonsoft.json.dll"), false)),
            "matches the file name"
        );
        assert!(m.matches(&frame("Shop.Internal.Run", None, false)));
        assert!(m.matches(&frame("anything", None, true)), "flags honoured");
        assert!(!m.matches(&frame("Shop.Place", Some("Shop.dll"), false)));
        let no_flags = FrameworkMatcher::new(&[], false).unwrap();
        assert!(!no_flags.matches(&frame("anything", None, true)));
        assert!(FrameworkMatcher::new(&vec!["a".into(); MAX_PATTERNS + 1], true).is_err());
        assert!(FrameworkMatcher::new(&["a".repeat(MAX_PATTERN_LEN + 1)], true).is_err());
    }
}
