import { FILE_EXISTS_SUFFIX, ImportProgress, ProfileVersionInfo } from '../api.types';
import { MockClock, systemMockClock } from './mock-clock';

/** Simulated files per opened path in a slow import (each path stands for a folder of this many files). */
export const SLOW_FILES_PER_PATH = 6;
/** Pretended size of each simulated file. */
export const SLOW_FILE_BYTES = 2_000_000;
/** Reason given for the files a cancelled import did not read (as the engine words it). */
export const NOT_IMPORTED_REASON = 'Not imported: the import was cancelled';

/** Where the browser demo pretends the bundled samples are installed. */
export const MOCK_DEMO_ROOT = '/demo/samples';
export const MOCK_DEMO_PATHS: readonly string[] = ['serilog-clef', 'serilog-json', 'serilog-text', 'log4net', 'csv'].map((d) => `${MOCK_DEMO_ROOT}/${d}`);

export function isMockDemoPath(path: string): boolean {
  return path === MOCK_DEMO_ROOT || path.startsWith(`${MOCK_DEMO_ROOT}/`);
}

export const IDLE_PROGRESS: ImportProgress = {
  running: false,
  done: 0,
  total: 0,
  cancelled: false,
  bytesDone: 0,
  bytesTotal: 0,
  current: null,
  elapsedMs: 0,
  etaMs: null,
};

/**
 * A slow import, file by file, that can be cancelled between files: the browser stand-in for the engine's
 * `ImportProgress`. `read` is called for every file that is read; files after a cancel are not.
 */
export class MockImportSim {
  private state: ImportProgress = IDLE_PROGRESS;
  private cancelRequested = false;

  constructor(
    private readonly msPerFile: number,
    private readonly clock: MockClock = systemMockClock,
  ) {}

  progress(): ImportProgress {
    return { ...this.state };
  }

  cancel(): void {
    if (this.state.running) this.cancelRequested = true;
  }

  /**
   * Reads `files` one after the other, telling `onProgress` (the import job's progress) after each step; resolves with
   * the files read and the ones skipped by a cancel.
   */
  async run(files: string[], read: (file: string) => void, onProgress: (p: ImportProgress) => void = () => undefined): Promise<{ read: string[]; skipped: string[] }> {
    const started = this.clock.now();
    this.cancelRequested = false;
    const total = files.length;
    this.state = { ...IDLE_PROGRESS, running: true, total, bytesTotal: total * SLOW_FILE_BYTES };
    const done: string[] = [];
    for (const file of files) {
      if (this.cancelRequested) break;
      this.state = { ...this.state, current: file };
      onProgress(this.progress());
      await this.clock.sleep(this.msPerFile);
      read(file);
      done.push(file);
      const elapsedMs = this.clock.now() - started;
      const bytesDone = done.length * SLOW_FILE_BYTES;
      const etaMs = Math.round((elapsedMs / bytesDone) * (this.state.bytesTotal - bytesDone));
      this.state = { ...this.state, done: done.length, bytesDone, elapsedMs, etaMs };
      onProgress(this.progress());
    }
    this.state = { ...this.state, running: false, current: null, cancelled: this.cancelRequested, elapsedMs: this.clock.now() - started, etaMs: null };
    return { read: done, skipped: files.slice(done.length) };
  }
}

/** The files a slow import pretends each opened path contains. */
export function slowImportFiles(paths: string[]): string[] {
  return paths.flatMap((p) => Array.from({ length: SLOW_FILES_PER_PATH }, (_, i) => `${p.replace(/[\\/]+$/, '')}/part-${i + 1}.log`));
}

/**
 * Files the browser demo has "written" (it writes nothing): a second save to the same path is refused
 * unless overwriting was confirmed, like the engine's guard. Each mock backend keeps its own.
 */
export class MockFiles {
  private readonly paths = new Set<string>();

  /** Throws the engine's FILE_EXISTS error for a known path without `overwrite`, else remembers the path. */
  write(path: string, overwrite: boolean | undefined): void {
    if (!overwrite && this.paths.has(path)) throw new Error(`${path}${FILE_EXISTS_SUFFIX}`);
    this.paths.add(path);
  }

  has(path: string): boolean {
    return this.paths.has(path);
  }

  clear(): void {
    this.paths.clear();
  }
}

/** Previous profile versions kept by the mock's `saveProfile`, like the shell's `history/` folder (newest first). */
export class MockProfileHistory {
  static readonly LIMIT = 20;
  private readonly versions: (ProfileVersionInfo & { toml: string })[] = [];
  private seq = 0;

  /** Archives `toml` (the profile being replaced). `now` is injectable for tests. */
  archive(toml: string, name: string, now = systemMockClock.now()): void {
    const at = new Date(now);
    const pad = (n: number, w = 2) => String(n).padStart(w, '0');
    const savedAt = `${at.getUTCFullYear()}${pad(at.getUTCMonth() + 1)}${pad(at.getUTCDate())}-${pad(at.getUTCHours())}${pad(at.getUTCMinutes())}${pad(at.getUTCSeconds())}${pad(at.getUTCMilliseconds(), 3)}`;
    this.versions.unshift({ file: `${savedAt}-${++this.seq}.debriefprofile.toml`, name, savedAt, savedAtMs: now, toml });
    this.versions.length = Math.min(this.versions.length, MockProfileHistory.LIMIT);
  }

  list(): ProfileVersionInfo[] {
    return this.versions.map(({ toml: _toml, ...info }) => info);
  }

  /** The text of one version; throws like the shell for an unknown or unsafe name. */
  get(file: string): { name: string; toml: string } {
    if (/[\\/]|\.\./.test(file)) throw new Error('invalid version');
    const v = this.versions.find((x) => x.file === file);
    if (!v) throw new Error(`no such version: ${file}`);
    return { name: v.name, toml: v.toml };
  }
}
