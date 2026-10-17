import { InjectionToken } from '@angular/core';
import { isTauri } from '../core/platform';

/** Reads the start of a file so the format wizard can show what IMT could not recognise. */
export abstract class FilePreview {
  /** At most `max` lines from the start of the file. Rejects with a message the user can act on. */
  abstract firstLines(path: string, max: number): Promise<string[]>;
}

/**
 * Desktop implementation. TODO: there is no backend command for this yet (it must read only files the
 * user opened, with a size limit); until it exists the wizard tells the user it is not supported.
 */
export class TauriFilePreview extends FilePreview {
  firstLines(): Promise<string[]> {
    return Promise.reject(new Error('Previewing files is not supported yet in the desktop app. Paste a Serilog output template instead.'));
  }
}

/** Browser demo: there is no file system, so it shows a typical Serilog plain-text log. */
export class SampleFilePreview extends FilePreview {
  readonly lines = [
    '2026-10-14 10:41:02.114 +00:00 [INF] Order 1042 accepted for customer 77',
    '2026-10-14 10:41:02.980 +00:00 [WRN] Payment gateway slow: 1500 ms',
    '2026-10-14 10:41:03.201 +00:00 [ERR] Payment capture failed for order 1042',
    '   at Payments.Capture() in Capture.cs:line 88',
    '2026-10-14 10:41:04.007 +00:00 [INF] Order 1043 accepted for customer 12',
  ];

  firstLines(_path: string, max: number): Promise<string[]> {
    return Promise.resolve(this.lines.slice(0, max));
  }
}

export const FILE_PREVIEW = new InjectionToken<FilePreview>('FILE_PREVIEW', {
  providedIn: 'root',
  factory: () => (isTauri() ? new TauriFilePreview() : new SampleFilePreview()),
});
