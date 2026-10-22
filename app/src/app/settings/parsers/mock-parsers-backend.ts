import { ParserDocDto, ParserDraftRequest, ParserImportCheckDto, ParserLibraryDto, ParserValidationDto } from '../../core/api.types';
import type { ImtBackend } from '../../core/backend';
import { CmdError } from '../../core/cmd-error';
import { MockBackend } from '../../core/mock/mock-backend';
import { MockParserLibrary, MockParserSource } from '../../core/mock/mock-parsers';
import { mockJobs, mockSteps } from '../../jobs/mock-jobs';
import type { ParserSaveOptions, ParsersBackend } from './parsers-backend';

/**
 * Browser stand-in for the parser library commands, over the mock workspace's library (`MockBackend.parsers`), so
 * the sources' parsers and "stale" follow what the user does. Over another backend it keeps a library of its own
 * without sources. Plausible, never the reference: the engine is.
 */
export class MockParsersBackend implements ParsersBackend {
  private readonly own = new MockParserLibrary();

  constructor(private readonly imt: ImtBackend) {}

  private get library(): MockParserLibrary {
    return this.imt instanceof MockBackend ? this.imt.parsers : this.own;
  }

  private sources(): MockParserSource[] {
    return this.imt instanceof MockBackend ? this.imt.parserSources() : [];
  }

  /** Like a command: resolves on a later task; a throw becomes a rejection. */
  private later<T>(work: () => T): Promise<T> {
    return new Promise<T>((resolve, reject) =>
      setTimeout(() => {
        try {
          resolve(work());
        } catch (e) {
          reject(e);
        }
      }, 10),
    );
  }

  list(): Promise<ParserLibraryDto> {
    return this.later(() => this.library.list(this.sources()));
  }

  get(id: string): Promise<ParserDocDto> {
    return this.later(() => this.library.get(id));
  }

  validate(toml: string): Promise<ParserValidationDto> {
    return this.later(() => this.library.validate(toml));
  }

  save(toml: string, options: ParserSaveOptions = {}): Promise<ParserLibraryDto> {
    return this.later(() => {
      this.library.save(toml, options.previousId ?? null, options.overwrite ?? false);
      return this.library.list(this.sources());
    });
  }

  duplicate(id: string): Promise<ParserDocDto> {
    return this.later(() => this.library.duplicate(id));
  }

  remove(id: string): Promise<ParserLibraryDto> {
    return this.later(() => {
      this.library.remove(id);
      return this.library.list(this.sources());
    });
  }

  setEnabled(id: string, enabled: boolean): Promise<ParserLibraryDto> {
    return this.later(() => {
      this.library.setEnabled(id, enabled);
      return this.library.list(this.sources());
    });
  }

  importCheck(path: string): Promise<ParserImportCheckDto> {
    return this.later(() => {
      if (!/\.toml$/i.test(path)) throw new CmdError({ code: 'invalid', message: `${path} is not a parser file (.toml)`, retryable: false });
      return this.library.importCheck(path);
    });
  }

  exportTo(id: string, path: string, overwrite = false): Promise<void> {
    return this.later(() => this.library.exportTo(id, path, overwrite));
  }

  draft(request: ParserDraftRequest): Promise<ParserDocDto> {
    return this.later(() => this.library.draft(request));
  }

  /** Like the engine: an import job re-reads the stale sources; the summary then names their new parsers. */
  reread(): Promise<number> {
    return this.later(() => this.job('Re-read files with the changed parsers', () => this.library.reread(this.sources())));
  }

  assign(sourceId: number, parserId: string | null): Promise<number> {
    return this.later(() => {
      const source = this.sources().find((s) => s.id === sourceId);
      if (!source) throw new CmdError({ code: 'notFound', message: 'that file is no longer open in this window', retryable: false });
      if (parserId !== null) this.library.get(parserId);
      return this.job(`Read ${source.name} again`, () => this.library.assign(source, parserId));
    });
  }

  private job(title: string, work: () => unknown): number {
    const job = mockJobs().start({ kind: 'import', title, cancellable: false }, async (ctx) => {
      await mockSteps(ctx, 50, 3);
      work();
      return { value: null, result: { type: 'workspace' } };
    });
    job.done.catch(() => undefined);
    return job.id;
  }
}
