import { homedir, platform } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

/** Paths the real-app smoke tests need. Pure: nothing here touches the disk. */

const here = dirname(fileURLToPath(import.meta.url));
export const E2E_DIR = resolve(here, '..');
export const APP_DIR = resolve(E2E_DIR, '..');
export const REPO_DIR = resolve(APP_DIR, '..');

const WINDOWS = platform() === 'win32';
/** Tauri's bundle identifier (`tauri.conf.json`): the name of Debrief's data and config folders. */
const IDENTIFIER = 'net.decryptology.debrief';

/**
 * The debug app binary: `DEBRIEF_E2E_APP`, else `<CARGO_TARGET_DIR or target>/debug/debrief-app[.exe]` (what
 * `npx tauri build --debug --no-bundle` produces).
 */
export function appBinary(env: NodeJS.ProcessEnv = process.env): string {
  if (env['DEBRIEF_E2E_APP']) return resolve(env['DEBRIEF_E2E_APP']);
  const target = env['CARGO_TARGET_DIR']
    ? resolve(REPO_DIR, env['CARGO_TARGET_DIR'])
    : join(REPO_DIR, 'target');
  return join(target, 'debug', WINDOWS ? 'debrief-app.exe' : 'debrief-app');
}

/** tauri-driver: `TAURI_DRIVER`, else the one `cargo install tauri-driver` put in `~/.cargo/bin`. */
export function tauriDriver(env: NodeJS.ProcessEnv = process.env): string {
  if (env['TAURI_DRIVER']) return resolve(env['TAURI_DRIVER']);
  const cargoHome = env['CARGO_HOME'] ?? join(homedir(), '.cargo');
  return join(cargoHome, 'bin', WINDOWS ? 'tauri-driver.exe' : 'tauri-driver');
}

/** The native WebDriver tauri-driver forwards to on Windows (fetched by `scripts/fetch-msedgedriver.mjs`). */
export function nativeDriver(env: NodeJS.ProcessEnv = process.env): string | null {
  if (env['DEBRIEF_E2E_NATIVE_DRIVER']) return resolve(env['DEBRIEF_E2E_NATIVE_DRIVER']);
  return WINDOWS ? join(E2E_DIR, '.drivers', 'msedgedriver.exe') : null;
}

/** The sample logs every smoke test opens. */
export const SAMPLES = join(REPO_DIR, 'samples', 'serilog-clef');

/**
 * Debrief's data folder (where the running marker lives): `$DEBRIEF_DATA_DIR/data` when the run relocates Debrief's folders
 * (debug builds honour it, see `app_dirs.rs`), else Tauri's `app_data_dir`: `%APPDATA%\<id>` on Windows,
 * `$XDG_DATA_HOME/<id>` (default `~/.local/share/<id>`) on Linux.
 */
export function dataDir(env: NodeJS.ProcessEnv = process.env): string {
  if (env['DEBRIEF_DATA_DIR']) return join(env['DEBRIEF_DATA_DIR'], 'data');
  if (WINDOWS) return join(env['APPDATA'] ?? join(homedir(), 'AppData', 'Roaming'), IDENTIFIER);
  return join(env['XDG_DATA_HOME'] ?? join(homedir(), '.local', 'share'), IDENTIFIER);
}

/** `running.marker`: present while Debrief runs; left behind by a crash (see `lifecycle.rs`). */
export function runningMarker(env: NodeJS.ProcessEnv = process.env): string {
  return join(dataDir(env), 'running.marker');
}

/**
 * The variable through which the launcher hands its per-run temp folder to the workers (they load this config
 * again and inherit the launcher's environment). Internal to the run: never set it yourself.
 */
export const RUN_DIR_VAR = 'DEBRIEF_E2E_RUN_DIR_INTERNAL';

/** Where a run keeps Debrief's folders (`DEBRIEF_DATA_DIR`) and, on Linux, the XDG folders WebKitGTK uses. */
export function runLayout(runDir: string): {
  debrief: string;
  xdg: { data: string; config: string; cache: string };
} {
  const xdg = join(runDir, 'xdg');
  return {
    debrief: join(runDir, 'debrief'),
    xdg: { data: join(xdg, 'data'), config: join(xdg, 'config'), cache: join(xdg, 'cache') },
  };
}

/** Screenshots and page sources of failed tests. */
export const ARTIFACTS_DIR = join(E2E_DIR, '.artifacts');
