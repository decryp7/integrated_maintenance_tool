// Generated from the Rust wire types by `cargo test -p debrief-app --lib generated_types` (ts-rs; ADR 0016).
// Do not edit: change the Rust type and run the test with DEBRIEF_WRITE_TYPES=1.

export type ErrorCode = "failed" | "invalid" | "notFound" | "notGranted" | "busy" | "cancelled" | "io" | "limit" | "unsupported" | "unavailable" | "fileExists" | "denied" | "missing" | "internal" | "noModel" | "noRuntime" | "noVision" | "engine" | "browser" | "contextOverflow" | "malformed" | "toolLoopLimit" | "modelFile" | "memory" | "network" | "tls" | "proxyAuth" | "proxy" | "offline" | "timeout" | "httpStatus" | "gated" | "integrity" | "disk" | "refused" | "selfTestFailed" | "notACapture" | "notAvailable" | "unreachable" | "noRelease" | "noPlatform" | "invalidManifest" | "signatureInvalid" | "tooLarge" | "diskFull" | "installFailed" | "untrusted" | "incompatible";

export type CmdErrorDto = { code: ErrorCode, 
/**
 * For the user, as is.
 */
message: string, 
/**
 * Trying again as is may succeed.
 */
retryable: boolean, 
/**
 * The request field the error is about (a form highlights it).
 */
field?: string, 
/**
 * The 1-based line of a text the error is about (a profile, a query).
 */
line?: number, 
/**
 * What to do next, for the user.
 */
hint?: string, 
/**
 * Technical detail (shown on demand, logged).
 */
detail?: string, };
