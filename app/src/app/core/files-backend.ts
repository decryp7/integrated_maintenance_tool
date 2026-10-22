import { inject, Injectable } from '@angular/core';
import { invoke } from './activity/ipc';
import { FileOpened, OpenFileRequest } from './api.types';
import { DebriefBackend } from './backend';
import { Logger, withFailureLogging } from './logger';
import { isTauri } from './platform';

/**
 * Opening an opened source outside Debrief (default program, editor at a line, file manager). The UI names the source by
 * id; the engine resolves and checks the path. Injected as a class token: Tauri in the desktop app, the mock in a browser.
 */
@Injectable({ providedIn: 'root', useFactory: () => createFilesBackend() })
export abstract class FilesBackend {
  /** Rejects with a message saying what to do instead (archive member, moved file, no line, a script...). */
  abstract openSourceFile(request: OpenFileRequest): Promise<FileOpened>;
}

/** Talks to the Rust shell (`open_source_file`). */
export class TauriFilesBackend extends FilesBackend {
  /** `call` is Tauri's `invoke`; tests pass a fake. */
  constructor(private readonly call: typeof invoke = invoke) {
    super();
  }

  openSourceFile(request: OpenFileRequest) {
    return this.call<FileOpened>('open_source_file', { request });
  }
}

/** True when a backend object also implements the files commands (the mock backend does). */
export function hasFiles(backend: unknown): backend is FilesBackend {
  return typeof (backend as Partial<FilesBackend> | null)?.openSourceFile === 'function';
}

/** A backend that refuses, for a workspace backend that cannot open files. */
export class UnsupportedFilesBackend extends FilesBackend {
  openSourceFile(): Promise<FileOpened> {
    return Promise.reject($localize`:@@core.files.unsupported:this backend cannot open files`);
  }
}

/**
 * The Tauri backend in the desktop app (refusals are expected answers, so they are logged, not reported as
 * failures); in a browser the mock workspace, which mimics the engine's checks.
 */
export function createFilesBackend(): FilesBackend {
  if (!isTauri()) {
    const workspace = inject(DebriefBackend);
    return hasFiles(workspace) ? workspace : new UnsupportedFilesBackend();
  }
  return withFailureLogging<FilesBackend>(new TauriFilesBackend(), inject(Logger));
}
