//! Which modules and frames belong to the system, the runtime or an allocator: the frames findings skip to find
//! the likely culprit. Pure name rules, so they are the same for dumps and UMDH logs.

/// The last component of a Windows or Unix path.
pub fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// Operating system, C runtime and loader modules (lower case, without extension).
const SYSTEM: &[&str] = &[
    "ntdll",
    "kernel32",
    "kernelbase",
    "user32",
    "win32u",
    "gdi32",
    "gdi32full",
    "combase",
    "ole32",
    "oleaut32",
    "rpcrt4",
    "sechost",
    "advapi32",
    "ws2_32",
    "mswsock",
    "msvcrt",
    "ucrtbase",
    "ucrtbased",
    "vcruntime140",
    "vcruntime140_1",
    "vcruntime140d",
    "msvcp140",
    "msvcp140d",
    "wow64",
    "wow64win",
    "wow64cpu",
    "shell32",
    "shlwapi",
    "bcrypt",
    "crypt32",
    "uxtheme",
    "dwmapi",
    "libc",
    "libpthread",
    "libstdc++",
    "libgcc_s",
    "ld-linux-x86-64",
    "libsystem_kernel",
    "libsystem_pthread",
    "libsystem_c",
    "libdyld",
    "dyld",
];

/// .NET runtime modules (managed frames of user code are in other modules).
const RUNTIME: &[&str] = &[
    "clr",
    "coreclr",
    "libcoreclr",
    "mscorwks",
    "clrjit",
    "libclrjit",
    "mscoreei",
    "mscoree",
    "hostfxr",
    "hostpolicy",
    "system.private.corelib",
    "mscorlib",
    "mscorlib.ni",
    "system.private.corelib.ni",
];

/// The module name without extension, lower case (`KERNELBASE.dll` → `kernelbase`).
fn stem(name: &str) -> String {
    let base = file_name(name).to_ascii_lowercase();
    // `libc.so.6`, `ntdll.dll`, `mscorlib.ni.dll`: drop the known extensions only.
    let mut s = base.as_str();
    for ext in [".dll", ".exe", ".sys", ".dylib"] {
        if let Some(t) = s.strip_suffix(ext) {
            s = t;
            break;
        }
    }
    if let Some(i) = s.find(".so").filter(|i| s.len() == i + 3 || s[i + 3..].starts_with('.')) {
        s = &s[..i];
    }
    s.to_string()
}

/// Whether `path` is a module of the operating system or the C runtime (by name, or under the Windows folder).
pub fn is_system_module(path: &str) -> bool {
    let lower = path.to_ascii_lowercase().replace('/', "\\");
    if lower.contains("\\windows\\system32\\") || lower.contains("\\windows\\syswow64\\") || lower.contains("\\windows\\winsxs\\") {
        return true;
    }
    SYSTEM.contains(&stem(path).as_str())
}

/// Whether `path` is a module of the .NET runtime.
pub fn is_runtime_module(path: &str) -> bool {
    let s = stem(path);
    RUNTIME.contains(&s.as_str()) || s.starts_with("system.private.")
}

/// Whether a frame in `module` (path or name) is not user code: system, C runtime or .NET runtime.
pub fn is_framework(module: &str) -> bool {
    is_system_module(module) || is_runtime_module(module)
}

/// Function names of heap allocators: UMDH stacks start inside them, and leaks are attributed to their caller.
const ALLOCATORS: &[&str] = &[
    "rtlallocateheap",
    "rtlpallocateheap",
    "rtlpallocateheapinternal",
    "rtlplowfragheapalloc",
    "rtlreallocateheap",
    "rtlpreallocateheap",
    "rtlpreallocateheapinternal",
    "rtldebugallocateheap",
    "heapalloc",
    "heaprealloc",
    "localalloc",
    "globalalloc",
    "virtualalloc",
    "malloc",
    "_malloc_base",
    "_malloc_dbg",
    "calloc",
    "_calloc_base",
    "realloc",
    "_realloc_base",
    "operator new",
    "operator new[]",
    "sysallocstring",
    "sysallocstringlen",
    "cotaskmemalloc",
];

/// Whether `function` is a heap allocator entry point.
pub fn is_allocator(function: &str) -> bool {
    let f = function.trim().to_ascii_lowercase();
    let f = f.rsplit("::").next().unwrap_or(&f);
    ALLOCATORS.contains(&f) || (f.starts_with("rtlp") && f.contains("heap")) || f.starts_with("??2@") || f.starts_with("??_u@")
}

/// Function names of waits on a lock (critical sections, SRW locks, mutexes, monitors).
const LOCK_WAITS: &[&str] = &[
    "rtlpwaitoncriticalsection",
    "rtlpentercriticalsectioncontended",
    "rtlentercriticalsection",
    "rtlacquiresrwlockexclusive",
    "rtlacquiresrwlockshared",
    "rtlpwaitonaddress",
    "rtlpwaitonaddresswithtimeout",
    "rtlsleepconditionvariablesrw",
    "rtlsleepconditionvariablecs",
    "sleepconditionvariablesrw",
    "sleepconditionvariablecs",
    "entercriticalsection",
    "acquiresrwlockexclusive",
    "acquiresrwlockshared",
    "pthread_mutex_lock",
    "__pthread_mutex_lock",
    "__lll_lock_wait",
    "pthread_cond_wait",
    "awarelock::enter",
    "awarelock::entercontended",
    "jit_monenter",
    "jit_monreliableenter",
    "monitor.enter",
    "monitor.reliableenter",
    "_os_unfair_lock_lock_slow",
];

/// Function names of waits on kernel objects (events, mutexes, handles).
const OBJECT_WAITS: &[&str] = &[
    "ntwaitforsingleobject",
    "zwwaitforsingleobject",
    "ntwaitformultipleobjects",
    "zwwaitformultipleobjects",
    "ntwaitforalertbythreadid",
    "waitforsingleobject",
    "waitforsingleobjectex",
    "waitformultipleobjects",
    "waitformultipleobjectsex",
    "msgwaitformultipleobjects",
    "msgwaitformultipleobjectsex",
    "ntdelayexecution",
    "sleepex",
    "futex_wait",
    "__futex_abstimed_wait_common",
];

/// What a thread blocked in `function` waits for, if it is a wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitKind {
    /// A lock (critical section, SRW lock, monitor, mutex).
    Lock,
    /// A kernel object or a sleep.
    Object,
}

fn bare(function: &str) -> String {
    let f = function.trim().to_ascii_lowercase();
    // `System.Threading.Monitor.Enter(System.Object)` → `monitor.enter`.
    let f = f.split('(').next().unwrap_or(&f).trim().to_string();
    let parts: Vec<&str> = f.split(['.', ':']).filter(|p| !p.is_empty()).collect();
    match parts.len() {
        0 => f,
        1 => parts[0].to_string(),
        n if f.contains("::") => format!("{}::{}", parts[n - 2], parts[n - 1]),
        n => format!("{}.{}", parts[n - 2], parts[n - 1]),
    }
}

/// Whether `function` waits, and for what.
pub fn wait_kind(function: &str) -> Option<WaitKind> {
    let full = function.trim().to_ascii_lowercase();
    let b = bare(function);
    let last = b.rsplit(['.', ':']).next().unwrap_or(&b).to_string();
    if LOCK_WAITS.iter().any(|w| *w == b || *w == last || *w == full) {
        Some(WaitKind::Lock)
    } else if OBJECT_WAITS.iter().any(|w| *w == b || *w == last || *w == full) {
        Some(WaitKind::Object)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_and_runtime_modules_are_recognised_by_name_or_folder() {
        assert!(is_system_module("C:\\Windows\\System32\\ntdll.dll"));
        assert!(is_system_module("KERNELBASE.dll"));
        assert!(is_system_module("c:/windows/syswow64/whatever.dll"));
        assert!(is_system_module("/lib/x86_64-linux-gnu/libc.so.6"));
        assert!(!is_system_module("C:\\Program Files\\Shop\\OrderService.exe"));
        assert!(is_runtime_module(
            "C:\\Program Files\\dotnet\\shared\\Microsoft.NETCore.App\\8.0.1\\coreclr.dll"
        ));
        assert!(is_runtime_module("System.Private.CoreLib.dll"));
        assert!(is_runtime_module("System.Private.Uri.dll"));
        assert!(is_framework("clr.dll") && is_framework("ucrtbase.dll") && !is_framework("Shop.Orders.dll"));
        assert_eq!(file_name("a/b\\c.dll"), "c.dll");
        assert_eq!(file_name("plain"), "plain");
    }

    #[test]
    fn allocators_and_waits_are_recognised() {
        for f in [
            "RtlAllocateHeap",
            "ntdll!x::malloc",
            "RtlpAllocateHeapInternal",
            "operator new",
            "??2@YAPEAX_K@Z",
        ] {
            assert!(is_allocator(f), "{f}");
        }
        assert!(!is_allocator("Shop::Cache::Add"));
        assert_eq!(wait_kind("RtlpWaitOnCriticalSection"), Some(WaitKind::Lock));
        assert_eq!(wait_kind("System.Threading.Monitor.Enter(System.Object)"), Some(WaitKind::Lock));
        assert_eq!(wait_kind("AwareLock::EnterContended"), Some(WaitKind::Lock));
        assert_eq!(wait_kind("NtWaitForSingleObject"), Some(WaitKind::Object));
        assert_eq!(wait_kind("Shop.Orders.Save"), None);
        assert_eq!(wait_kind(""), None);
    }
}
