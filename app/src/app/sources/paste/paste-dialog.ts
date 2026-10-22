import { ChangeDetectionStrategy, Component, computed, inject, input, OnInit, signal } from '@angular/core';
import { WorkspaceStore } from '../../core/workspace.store';
import { formatBytes } from '../../shared/format/units';
import { errorMessage } from '../../tools/tool-errors';
import { DIALOG_REF } from '../../ui/dialog.service';
import { PasteBackend } from './paste-backend';
import { byteLength, nameToSend, overCap, PASTE_MAX_BYTES } from './paste-limits';

/** The name a pasted log gets until the user types another. */
export const DEFAULT_PASTE_NAME = 'Pasted log';

/**
 * Edit ▸ "Paste as log…": adds pasted text to the project as a log file and opens it. The text is taken from the
 * clipboard when it can be read (or comes in as `text`, from a drop); the engine names the file, validates the text and
 * imports it, this dialog only refuses early what is over the cap.
 */
@Component({
  selector: 'debrief-paste-dialog',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <p class="intro" i18n="@@sources.paste.dialog.intro">The text is saved in the project as a log file and opened like any other log.</p>
    <label class="field">
      <span i18n="@@sources.paste.dialog.name">Name</span>
      <input class="input" type="text" data-testid="paste-name" [value]="name()" (input)="name.set($any($event.target).value)" />
    </label>
    <label class="field">
      <span i18n="@@sources.paste.dialog.text">Log text</span>
      <textarea class="input mono" rows="14" spellcheck="false" data-testid="paste-text" [value]="content()" [attr.aria-invalid]="tooBig()" (input)="content.set($any($event.target).value)"></textarea>
    </label>
    <p class="count" [class.over]="tooBig()" data-testid="paste-count" aria-live="polite" i18n="@@sources.paste.dialog.count">{{ used() }} of {{ limit }} used</p>
    @if (tooBig()) {
      <p class="err" role="alert" data-testid="paste-over" i18n="@@sources.paste.dialog.over">This is over {{ limit }}. Save it as a file and open that instead.</p>
    }
    @if (error(); as e) { <p class="err" role="alert" data-testid="paste-error">{{ e }}</p> }
    <div class="foot">
      <button type="button" class="btn ghost" data-testid="paste-cancel" (click)="ref.close()" i18n="@@sources.paste.dialog.cancel">Cancel</button>
      <button type="button" class="btn primary" data-testid="paste-add" [disabled]="!canAdd()" (click)="add()" i18n="@@sources.paste.dialog.add">Add</button>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 10px; }
    p { margin: 0; }
    .field { display: flex; flex-direction: column; gap: 4px; }
    textarea { width: 100%; box-sizing: border-box; resize: vertical; font-family: var(--font-mono); font-size: var(--fs-sm); }
    .count { color: var(--muted); font-size: var(--fs-xs); }
    .count.over, .err { color: var(--danger-text); }
    .foot { display: flex; justify-content: flex-end; gap: 8px; }
  `,
})
export class PasteDialog implements OnInit {
  protected readonly ref = inject(DIALOG_REF);
  private readonly backend = inject(PasteBackend);
  private readonly workspace = inject(WorkspaceStore);
  /** Text to start with (a drop); without it the clipboard is tried. */
  readonly text = input<string | null>(null);
  protected readonly name = signal(DEFAULT_PASTE_NAME);
  protected readonly content = signal('');
  protected readonly error = signal<string | null>(null);
  protected readonly busy = signal(false);
  protected readonly limit = formatBytes(PASTE_MAX_BYTES);
  protected readonly bytes = computed(() => byteLength(this.content()));
  protected readonly used = computed(() => formatBytes(this.bytes()));
  protected readonly tooBig = computed(() => overCap(this.bytes()));
  protected readonly canAdd = computed(() => !this.busy() && !this.tooBig() && this.content().trim().length > 0);

  ngOnInit(): void {
    const given = this.text();
    if (given !== null) this.content.set(given);
    else void this.fromClipboard();
  }

  /** Pre-fills from the clipboard when it is readable and the user has not typed meanwhile; any failure is ignored. */
  private async fromClipboard(): Promise<void> {
    try {
      const clip = await navigator.clipboard.readText();
      if (clip && !this.content()) this.content.set(clip);
    } catch {
      // Not allowed or not available: the user pastes into the box.
    }
  }

  /** Sends the text, applies the new workspace and closes; a refusal stays in the dialog. */
  protected async add(): Promise<void> {
    if (!this.canAdd()) return;
    this.busy.set(true);
    this.error.set(null);
    try {
      const summary = await this.backend.pasteLog(this.content(), nameToSend(this.name()));
      this.workspace.applyImported(summary);
      this.ref.close(true);
    } catch (e) {
      this.error.set(errorMessage(e));
      this.busy.set(false);
    }
  }
}
