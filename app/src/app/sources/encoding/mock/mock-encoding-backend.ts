import { EncodingOption } from '../../../core/api.types';
import type { ImtBackend } from '../../../core/backend';
import { CmdError } from '../../../core/cmd-error';
import { MockBackend } from '../../../core/mock/mock-backend';
import { mockJobs, mockSteps } from '../../../jobs/mock-jobs';
import type { EncodingBackend } from '../encoding-backend';

/** The engine's choosable encodings. Browser development and UI tests only. */
export const MOCK_ENCODINGS: readonly EncodingOption[] = [
  { label: 'UTF-8', name: 'Unicode (UTF-8)' },
  { label: 'UTF-16LE', name: 'Unicode (UTF-16 little-endian)' },
  { label: 'UTF-16BE', name: 'Unicode (UTF-16 big-endian)' },
  { label: 'windows-1252', name: 'Western European (Windows-1252)' },
  { label: 'windows-1250', name: 'Central European (Windows-1250)' },
  { label: 'windows-1251', name: 'Cyrillic (Windows-1251)' },
  { label: 'windows-1253', name: 'Greek (Windows-1253)' },
  { label: 'windows-1254', name: 'Turkish (Windows-1254)' },
  { label: 'windows-1255', name: 'Hebrew (Windows-1255)' },
  { label: 'windows-1256', name: 'Arabic (Windows-1256)' },
  { label: 'windows-1257', name: 'Baltic (Windows-1257)' },
  { label: 'windows-1258', name: 'Vietnamese (Windows-1258)' },
  { label: 'Shift_JIS', name: 'Japanese (Shift_JIS)' },
  { label: 'EUC-JP', name: 'Japanese (EUC-JP)' },
  { label: 'GBK', name: 'Simplified Chinese (GBK)' },
  { label: 'gb18030', name: 'Chinese (GB18030)' },
  { label: 'Big5', name: 'Traditional Chinese (Big5)' },
  { label: 'EUC-KR', name: 'Korean (EUC-KR)' },
  { label: 'KOI8-R', name: 'Russian (KOI8-R)' },
  { label: 'ISO-8859-2', name: 'Central European (ISO-8859-2)' },
  { label: 'ISO-8859-15', name: 'Western European (ISO-8859-15)' },
];

/**
 * Browser stand-in for the encoding commands, over the mock workspace (which records the choice on the source, so the
 * Explorer shows it). Plausible, never the reference: the engine is.
 */
export class MockEncodingBackend implements EncodingBackend {
  /** The encoding chosen per source id (what the engine would read it with). */
  readonly chosen = new Map<number, string | null>();

  constructor(private readonly imt: ImtBackend) {}

  options(): Promise<EncodingOption[]> {
    return Promise.resolve(MOCK_ENCODINGS.map((o) => ({ ...o })));
  }

  /** Like a command: resolves on a later task; a throw becomes a rejection. */
  setSourceEncoding(sourceId: number, encoding: string | null): Promise<number> {
    return new Promise<number>((resolve, reject) =>
      setTimeout(() => {
        try {
          if (!(this.imt instanceof MockBackend)) throw new CmdError({ code: 'invalid', message: 'this backend cannot read a file again', retryable: false });
          const label = encoding === 'auto' ? null : encoding;
          if (label !== null && !MOCK_ENCODINGS.some((o) => o.label === label)) {
            throw new CmdError({ code: 'invalid', message: `unknown encoding "${label}"`, retryable: false });
          }
          const mock = this.imt;
          const name = mock.setSourceEncoding(sourceId, label);
          this.chosen.set(sourceId, label);
          const job = mockJobs().start({ kind: 'import', title: `Read ${name} again`, cancellable: false }, async (ctx) => {
            await mockSteps(ctx, 50, 3);
            return { value: null, result: { type: 'workspace' } };
          });
          job.done.catch(() => undefined);
          resolve(job.id);
        } catch (e) {
          reject(e);
        }
      }, 10),
    );
  }
}
