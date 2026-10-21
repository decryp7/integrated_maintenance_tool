import { computed, inject, Injectable, Injector, signal } from '@angular/core';
import { isUnsupported, sentenceText } from '../../core/cmd-error';
import { WorkspaceStore } from '../../core/workspace.store';
import { ProfileEdits } from '../../profile/profile-edits';
import { errorMessage } from '../../tools/tool-errors';
import { DialogService } from '../../ui/dialog.service';
import { ShareBackend } from '../share-backend';
import { readRedact } from './redact-section';
import { RedactionDialog } from './redaction-dialog';

/** The window's redacted view (switched in the engine) and the profile's redaction defaults. */
@Injectable({ providedIn: 'root' })
export class RedactionService {
  private readonly store = inject(WorkspaceStore);
  private readonly injector = inject(Injector);
  /** Resolved on use, so hosts that never share (e.g. the banners in tests) need no share backend. */
  private get share(): ShareBackend {
    return this.injector.get(ShareBackend);
  }
  private readonly edits = inject(ProfileEdits);
  private readonly dialogs = inject(DialogService);

  /** True while pages, facets and context are redacted (as the engine reports it). */
  readonly on = computed(() => this.store.summary()?.redactedView === true);
  readonly busy = signal(false);

  /** Switches the redacted view; the grid, facets and context reload with the returned summary. */
  async set(enabled: boolean): Promise<boolean> {
    this.busy.set(true);
    try {
      this.store.applySummary(await this.share.setRedactedView(enabled));
      return true;
    } catch (e) {
      // Not offered by the log store (yet): a notice that says so, not an error.
      if (isUnsupported(e)) this.store.setNotice(sentenceText(e));
      else this.store.setError($localize`:@@share.redactionService.switchFailed:Could not switch the redacted view: ${errorMessage(e)}:error:`);
      return false;
    } finally {
      this.busy.set(false);
    }
  }

  toggle(): Promise<boolean> {
    return this.set(!this.on());
  }

  /** Whether exports, reports and bundles start redacted (`[redact] on_export`); false when the profile cannot be read. */
  async onExportDefault(): Promise<boolean> {
    try {
      return readRedact((await this.edits.current()).model).onExport;
    } catch {
      return false;
    }
  }

  async openSettings(): Promise<void> {
    await this.dialogs.open(RedactionDialog, { title: $localize`:@@share.redactionService.dialogTitle:Redaction settings`, width: 640 }).closed;
  }
}
