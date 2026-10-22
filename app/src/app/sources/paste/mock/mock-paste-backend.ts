import { WorkspaceSummary } from '../../../core/api.types';
import type { ImtBackend } from '../../../core/backend';
import { CmdError } from '../../../core/cmd-error';
import { MockBackend } from '../../../core/mock/mock-backend';
import type { PasteBackend } from '../paste-backend';
import { byteLength, overCap, safeLogName, uniqueName } from '../paste-limits';

/**
 * Browser stand-in for `paste_log`, over the mock workspace: validates like the engine, makes the name safe and unique,
 * and adds a source with the text's lines as events. Plausible, never the reference: the engine is.
 */
export class MockPasteBackend implements PasteBackend {
  constructor(private readonly imt: ImtBackend) {}

  async pasteLog(text: string, name: string | null): Promise<WorkspaceSummary> {
    if (!(this.imt instanceof MockBackend)) throw new CmdError({ code: 'invalid', message: 'this backend cannot add pasted text', retryable: false });
    if (!text.trim()) throw new CmdError({ code: 'invalid', message: 'there is no text to add', retryable: false });
    if (overCap(byteLength(text))) {
      throw new CmdError({ code: 'invalid', message: 'the text is over 8 MiB; save it as a file and open that instead', retryable: false });
    }
    const taken = new Set((await this.imt.workspaceSummary()).sources.map((s) => s.name));
    return this.imt.addPastedSource(uniqueName(safeLogName(name), taken), text);
  }
}
