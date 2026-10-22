import type { ErrorCode } from '../generated/errors';

/**
 * Severity of a failure: mistakes the user can fix (validation, query syntax, missing files, rights,
 * cancellation) are warnings; anything else is an unexpected error. Crashes are fatal (Rust only).
 */
const USER_ERROR =
  /\binvalid\b|\bnot valid\b|syntax|parse error|unexpected (token|character|end)|does not compile|no such file|not found|cannot find|already (exists|open)|access (is )?denied|permission|requires? (elevat|administrator)|cancel+ed|aborted|not allowed|must (be|not|end|start)|too (long|large|many)|unsupported|empty|expected |regex|cannot export there|\bnot an? \w/i;

function text(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  try {
    return JSON.stringify(e) ?? String(e);
  } catch {
    return String(e);
  }
}

/**
 * Whether each engine error code is an expected outcome (the user's input, choices or environment: a cleared job, a
 * refused permission, an offline machine, a missing model) rather than IMT failing. A `Record` over every code, so a
 * new engine code does not compile until it is classified here.
 */
const EXPECTED: Record<ErrorCode, boolean> = {
  failed: false,
  invalid: true,
  notFound: true,
  notGranted: true,
  busy: true,
  cancelled: true,
  io: false,
  limit: true,
  unsupported: true,
  unavailable: true,
  fileExists: true,
  denied: true,
  missing: true,
  internal: false,
  noModel: true,
  noRuntime: true,
  noVision: true,
  engine: false,
  // A page of the research or agent browser failing (not open, slow, changed) is the web's, not an IMT bug.
  browser: true,
  contextOverflow: true,
  malformed: false,
  toolLoopLimit: true,
  modelFile: false,
  memory: true,
  network: true,
  tls: true,
  proxyAuth: true,
  proxy: true,
  offline: true,
  timeout: true,
  httpStatus: false,
  gated: true,
  integrity: false,
  disk: false,
  refused: true,
  // A new inference runtime that does not work with the user's model here: the previous one stays, nothing broke.
  selfTestFailed: true,
  notACapture: true,
  notAvailable: true,
  unreachable: true,
  noRelease: true,
  noPlatform: true,
  invalidManifest: true,
  signatureInvalid: true,
  tooLarge: true,
  diskFull: true,
  installFailed: false,
  // Plugin packages: a package that is not trusted or not for this IMT is the user's to resolve.
  untrusted: true,
  incompatible: true,
};

/** True for a failure the user can fix themselves (shown as is, logged as a warning). */
export function isUserError(error: unknown): boolean {
  if (typeof DOMException !== 'undefined' && error instanceof DOMException && error.name === 'AbortError') return true;
  // A typed engine error says what it is; the wording is only a fallback for errors without an engine code (older
  // or foreign codes such as `E_REGEX` are not the engine's and say nothing about severity).
  const code = (error as { code?: unknown } | null)?.code;
  if (typeof code === 'string' && Object.hasOwn(EXPECTED, code)) return EXPECTED[code as ErrorCode];
  return USER_ERROR.test(text(error));
}
