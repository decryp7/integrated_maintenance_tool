//! Exception codes decoded with a name (as debuggers show it) and what it usually means, so a crash finding
//! leads with the cause instead of a number.

/// A known exception code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExceptionCode {
    pub code: u32,
    pub name: &'static str,
    /// One sentence for the user.
    pub meaning: &'static str,
}

const fn e(code: u32, name: &'static str, meaning: &'static str) -> ExceptionCode {
    ExceptionCode { code, name, meaning }
}

/// The codes IMT knows (NTSTATUS values, the C++ and CLR exception codes).
pub const CODES: &[ExceptionCode] = &[
    e(0xC000_0005, "ACCESS_VIOLATION", "The code read, wrote or executed memory it may not touch: a null or dangling pointer, a buffer overrun or freed memory."),
    e(0xC000_0006, "IN_PAGE_ERROR", "Memory backed by a file could not be paged in: the file or the network share went away, or the disk failed."),
    e(0xC000_0008, "INVALID_HANDLE", "A handle that was already closed or never valid was used."),
    e(0xC000_0017, "NO_MEMORY", "An allocation failed: the process ran out of memory or address space."),
    e(0xC000_001D, "ILLEGAL_INSTRUCTION", "The CPU met an instruction it cannot run: corrupted code, a jump into data, or an instruction set the CPU lacks."),
    e(0xC000_0025, "NONCONTINUABLE_EXCEPTION", "Execution was resumed after an exception that does not allow it."),
    e(0xC000_0026, "INVALID_DISPOSITION", "An exception handler returned an invalid value."),
    e(0xC000_008C, "ARRAY_BOUNDS_EXCEEDED", "An array index was out of bounds (hardware bounds check)."),
    e(0xC000_008D, "FLT_DENORMAL_OPERAND", "A floating-point operation used a denormal value with the exception unmasked."),
    e(0xC000_008E, "FLT_DIVIDE_BY_ZERO", "A floating-point division by zero with the exception unmasked."),
    e(0xC000_008F, "FLT_INEXACT_RESULT", "A floating-point result could not be represented exactly, with the exception unmasked."),
    e(0xC000_0090, "FLT_INVALID_OPERATION", "An invalid floating-point operation with the exception unmasked."),
    e(0xC000_0091, "FLT_OVERFLOW", "A floating-point overflow with the exception unmasked."),
    e(0xC000_0092, "FLT_STACK_CHECK", "The x87 floating-point stack overflowed or underflowed."),
    e(0xC000_0093, "FLT_UNDERFLOW", "A floating-point underflow with the exception unmasked."),
    e(0xC000_0094, "INT_DIVIDE_BY_ZERO", "An integer was divided by zero."),
    e(0xC000_0095, "INT_OVERFLOW", "An integer operation overflowed (checked arithmetic)."),
    e(0xC000_0096, "PRIV_INSTRUCTION", "A privileged instruction was executed in user mode."),
    e(0xC000_00FD, "STACK_OVERFLOW", "The thread's stack is exhausted: usually unbounded recursion, sometimes very large stack allocations."),
    e(0xC000_0135, "DLL_NOT_FOUND", "A DLL the program needs could not be found when it started."),
    e(0xC000_0139, "ENTRYPOINT_NOT_FOUND", "A function the program imports is missing from the DLL it found: a version mismatch."),
    e(0xC000_0142, "DLL_INIT_FAILED", "A DLL's initialisation failed, often because the desktop or a resource it needs is unavailable."),
    e(0xC000_013A, "CONTROL_C_EXIT", "The program was stopped with Ctrl+C or Ctrl+Break."),
    e(0xC000_0194, "POSSIBLE_DEADLOCK", "A critical section wait timed out: a likely deadlock."),
    e(0xC000_0374, "HEAP_CORRUPTION", "The heap manager found its data structures corrupted: a buffer overrun, a double free or a use after free, usually earlier than the crash."),
    e(0xC000_0409, "STACK_BUFFER_OVERRUN", "A security check (/GS cookie or __fastfail) stopped the process: a stack buffer overrun or a fail-fast call; parameter 0 is the fast-fail code."),
    e(0xC000_0420, "ASSERTION_FAILURE", "An assertion in the code failed."),
    e(0xC000_0602, "FAIL_FAST_EXCEPTION", "The program asked to be terminated immediately (fail fast)."),
    e(0xC000_041D, "FATAL_USER_CALLBACK_EXCEPTION", "An exception escaped a user-mode callback from the kernel (window procedure, hook)."),
    e(0xC015_000F, "SXS_EARLY_DEACTIVATION", "An activation context was deactivated out of order."),
    e(0xC06D_007E, "DELAYLOAD_MODULE_NOT_FOUND", "A delay-loaded DLL could not be found when it was first used."),
    e(0xC06D_007F, "DELAYLOAD_PROC_NOT_FOUND", "A delay-loaded function is missing from its DLL."),
    e(0x8000_0001, "GUARD_PAGE_VIOLATION", "A guard page was touched (normally handled when a stack grows)."),
    e(0x8000_0002, "DATATYPE_MISALIGNMENT", "Misaligned data was accessed where alignment is required."),
    e(0x8000_0003, "BREAKPOINT", "A breakpoint instruction was hit (a debug break, an assertion, or __debugbreak)."),
    e(0x8000_0004, "SINGLE_STEP", "A single-step trap (a debugger was stepping)."),
    e(0x4000_001F, "WX86_BREAKPOINT", "A breakpoint in a 32-bit process under WOW64."),
    e(0x4000_0015, "FATAL_APP_EXIT", "The C runtime ended the program (abort())."),
    e(0x406D_1388, "SET_THREAD_NAME", "A thread was named (not an error)."),
    e(0xE06D_7363, "CPP_EH_EXCEPTION", "A C++ exception was thrown and not caught."),
    e(0xE043_4352, "CLR_EXCEPTION", "A .NET exception was thrown and not handled (the managed exception object has the details)."),
    e(0xE043_4F4D, "CLR_EXCEPTION", "A .NET exception was thrown and not handled (the managed exception object has the details)."),
    e(0xE053_534F, "CLR_STACK_OVERFLOW", "The .NET runtime detected a stack overflow."),
];

/// The known code `code`, if any.
pub fn lookup(code: u32) -> Option<&'static ExceptionCode> {
    CODES.iter().find(|c| c.code == code)
}

/// The name of `code` (`ACCESS_VIOLATION`), or its hex value when unknown.
pub fn name(code: u32) -> String {
    lookup(code).map_or_else(|| hex_code(code), |c| c.name.to_string())
}

/// `0xC0000005`.
pub fn hex_code(code: u32) -> String {
    format!("0x{code:08X}")
}

/// What an access violation did (`ExceptionInformation[0]`) and where (`[1]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Access {
    pub kind: AccessKind,
    pub address: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    Read,
    Write,
    Execute,
    Other,
}

impl AccessKind {
    pub fn verb(self) -> &'static str {
        match self {
            AccessKind::Read => "reading",
            AccessKind::Write => "writing",
            AccessKind::Execute => "executing",
            AccessKind::Other => "accessing",
        }
    }
}

/// The access of an access violation or in-page error.
pub fn access(code: u32, parameters: &[u64]) -> Option<Access> {
    if !matches!(code, 0xC000_0005 | 0xC000_0006) || parameters.len() < 2 {
        return None;
    }
    let kind = match parameters[0] {
        0 => AccessKind::Read,
        1 => AccessKind::Write,
        8 => AccessKind::Execute,
        _ => AccessKind::Other,
    };
    Some(Access {
        kind,
        address: parameters[1],
    })
}

/// Fast-fail codes (`STACK_BUFFER_OVERRUN`'s parameter 0) with their meaning.
pub fn fast_fail(code: u64) -> Option<&'static str> {
    Some(match code {
        2 => "FAST_FAIL_STACK_COOKIE_CHECK_FAILURE: a stack buffer overrun overwrote the /GS cookie",
        5 => "FAST_FAIL_INVALID_ARG: the C runtime's invalid parameter handler",
        7 => "FAST_FAIL_FATAL_APP_EXIT: abort() or a fatal application exit",
        10 => "FAST_FAIL_INVALID_SET_OF_CONTEXT: an invalid context was set",
        13 => "FAST_FAIL_GUARD_ICALL_CHECK_FAILURE: Control Flow Guard stopped an indirect call to an invalid target",
        14 => "FAST_FAIL_INVALID_LONGJUMP_TARGET",
        18 => "FAST_FAIL_ETW_CORRUPTION",
        23 => "FAST_FAIL_INVALID_FILE_OPERATION",
        37 => "FAST_FAIL_INVALID_IAT",
        38 => "FAST_FAIL_HEAP_METADATA_CORRUPTION: the heap's own data is corrupted",
        39 => "FAST_FAIL_PAYLOAD_RESTRICTION_VIOLATION",
        57 => "FAST_FAIL_UNHANDLED_LSS_EXCEPTON",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_named_with_a_meaning_and_unique() {
        let av = lookup(0xC000_0005).unwrap();
        assert_eq!(av.name, "ACCESS_VIOLATION");
        assert!(av.meaning.contains("pointer"));
        assert_eq!(name(0x1234_5678), "0x12345678");
        assert_eq!(name(0xC000_00FD), "STACK_OVERFLOW");
        let mut seen = std::collections::HashSet::new();
        assert!(CODES.iter().all(|c| seen.insert(c.code)), "duplicate code");
        assert!(CODES.iter().all(|c| !c.meaning.is_empty() && c.meaning.ends_with('.')));
    }

    #[test]
    fn access_violations_say_what_they_did_where() {
        let a = access(0xC000_0005, &[1, 0x10]).unwrap();
        assert_eq!((a.kind, a.address), (AccessKind::Write, 0x10));
        assert_eq!(access(0xC000_0005, &[8, 0]).unwrap().kind.verb(), "executing");
        assert_eq!(access(0xC000_0005, &[0, 0]).unwrap().kind.verb(), "reading");
        assert_eq!(access(0xC000_0005, &[3, 0]).unwrap().kind.verb(), "accessing");
        assert!(access(0xC000_0005, &[0]).is_none());
        assert!(access(0xE06D_7363, &[0, 1]).is_none());
        assert!(fast_fail(2).unwrap().contains("cookie"));
        assert!(fast_fail(999).is_none());
    }
}
