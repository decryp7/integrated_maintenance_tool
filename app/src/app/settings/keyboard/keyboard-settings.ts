import { ChangeDetectionStrategy, Component, computed, DOCUMENT, effect, ElementRef, inject, signal, viewChild } from '@angular/core';
import { CommandRegistry } from '../../extensions/command-registry';
import { formatKeybinding } from '../../extensions/keybinding';
import { KeybindingOverridesStore } from '../../extensions/keybinding-overrides.store';
import { findConflicts, Occupant, recordKey, withOverride, withoutOverride } from '../../extensions/keybinding-overrides';
import { FIXED_KEYS } from '../../help/shortcuts';
import { shortcutRows } from './shortcut-rows';

/** A recorded binding another command or a fixed key already uses, waiting for the user's decision. */
interface PendingConflict {
  id: string;
  title: string;
  binding: string;
  occupants: Occupant[];
}

/**
 * Settings ▸ Keyboard shortcuts: a searchable table of every command with its key and its default. Pick "Change",
 * press the new keys (Esc cancels, Backspace clears); a key another command or a fixed key uses asks first
 * (Replace or Cancel). Changes apply at once everywhere (menus, palette, tooltips, the cheat sheet) and are kept in
 * the browser like the other interface preferences.
 */
@Component({
  selector: 'imt-keyboard-settings',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="bar">
      <input class="input search" type="search" aria-label="Search shortcuts" i18n-aria-label="@@settings.keyboard.search.label" placeholder="Search shortcuts…" i18n-placeholder="@@settings.keyboard.search.placeholder" [value]="query()" (input)="query.set($any($event.target).value)" />
      <button class="btn" type="button" [disabled]="!hasOverrides()" (click)="resetAll()" i18n="@@settings.keyboard.resetAll">Reset all to defaults</button>
    </div>
    <p class="desc" i18n="@@settings.keyboard.help">Choose Change, then press the new keys. Esc cancels; Backspace removes the key. Keys that only work in some situations are marked.</p>
    <p class="status" role="status" aria-live="polite" data-testid="shortcut-status">{{ status() }}</p>

    @if (pending(); as p) {
      <div class="conflict" role="alertdialog" aria-labelledby="imt-kb-conflict" data-testid="shortcut-conflict">
        <span id="imt-kb-conflict">{{ conflictText(p) }}</span>
        @if (canReplace(p)) {
          <button class="btn primary" type="button" data-testid="shortcut-replace" (click)="replace(p)" i18n="@@settings.keyboard.replace">Replace</button>
        }
        <button #cancelConflict class="btn" type="button" data-testid="shortcut-cancel" (click)="cancelPending()" i18n="@@settings.keyboard.cancel">Cancel</button>
      </div>
    }

    <table class="table">
      <caption class="sr-only" i18n="@@settings.keyboard.caption">Commands and their keyboard shortcuts</caption>
      <thead>
        <tr>
          <th scope="col" i18n="@@settings.keyboard.col.command">Command</th>
          <th scope="col" i18n="@@settings.keyboard.col.category">Category</th>
          <th scope="col" i18n="@@settings.keyboard.col.key">Shortcut</th>
          <th scope="col" i18n="@@settings.keyboard.col.default">Default</th>
          <th scope="col"><span class="sr-only" i18n="@@settings.keyboard.col.actions">Actions</span></th>
        </tr>
      </thead>
      <tbody>
        @for (r of rows(); track r.id) {
          <tr [attr.data-command]="r.id" [class.changed]="r.changed">
            <th scope="row" class="title">
              {{ r.title }}
              @if (r.scoped) {
                <span class="scoped" title="Only works in some situations" i18n-title="@@settings.keyboard.scoped.title" i18n="@@settings.keyboard.scoped">(when available)</span>
              }
            </th>
            <td class="cat">{{ r.category }}</td>
            <td>
              <button
                class="key"
                type="button"
                [class.recording]="recording() === r.id"
                [attr.aria-label]="changeLabel(r.title, r.keys)"
                [attr.aria-pressed]="recording() === r.id"
                (click)="toggleRecording(r.id, r.title)"
                (blur)="stopOnBlur(r.id)"
              >
                @if (recording() === r.id) {
                  <span i18n="@@settings.keyboard.pressKeys">Press keys…</span>
                } @else if (r.keys) {
                  <kbd>{{ r.keys }}</kbd>
                } @else {
                  <span class="none" i18n="@@settings.keyboard.none">None</span>
                }
              </button>
            </td>
            <td class="def">
              @if (r.defaultKeys) {
                <kbd>{{ r.defaultKeys }}</kbd>
              } @else {
                <span class="none" i18n="@@settings.keyboard.none">None</span>
              }
            </td>
            <td class="act">
              <button class="btn ghost" type="button" [disabled]="!r.changed" [attr.aria-label]="resetLabel(r.title)" (click)="reset(r.id, r.title)" i18n="@@settings.keyboard.reset">Reset</button>
            </td>
          </tr>
        } @empty {
          <tr><td colspan="5" class="none" i18n="@@settings.keyboard.noMatch">No command matches “{{ query() }}”.</td></tr>
        }
      </tbody>
    </table>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: var(--s-2); }
    .bar { display: flex; gap: var(--s-2); }
    .search { flex: 1; }
    .desc, .status { margin: 0; color: var(--muted); line-height: 1.45; }
    .status { min-height: 1.2em; color: var(--text-2); }
    .conflict { display: flex; align-items: center; gap: var(--s-2); padding: var(--s-2); border: 1px solid var(--warn-text); border-radius: var(--r-2); background: var(--bg-3); color: var(--text); }
    .conflict span { flex: 1; }
    .table { width: 100%; border-collapse: collapse; }
    th, td { text-align: start; padding: 4px 6px; border-bottom: 1px solid var(--border); vertical-align: middle; }
    thead th { position: sticky; top: 0; background: var(--bg-1); font-size: var(--fs-sm); color: var(--muted); font-weight: 600; }
    tbody th.title { font-weight: 400; color: var(--text); }
    .cat, .def { color: var(--text-2); }
    .scoped { margin-inline-start: 6px; font-size: var(--fs-xs); color: var(--muted); }
    .none { color: var(--muted); }
    tr.changed .key kbd { border-color: var(--accent); }
    kbd { font-family: var(--font-mono); font-size: var(--fs-sm); padding: 1px 6px; border-radius: 4px; border: 1px solid var(--border-3); background: var(--bg-3); color: var(--text); }
    .key { min-width: 7em; text-align: start; padding: 2px 4px; border: 1px solid transparent; border-radius: var(--r-2); background: transparent; color: var(--text); cursor: pointer; }
    .key:hover { border-color: var(--border-3); background: var(--bg-hover); }
    .key.recording { border-color: var(--accent); background: var(--accent-soft); }
    .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
  `,
})
export class KeyboardSettings {
  private readonly registry = inject(CommandRegistry);
  private readonly store = inject(KeybindingOverridesStore);
  private readonly doc = inject(DOCUMENT);
  private readonly cancelButton = viewChild<ElementRef<HTMLButtonElement>>('cancelConflict');
  private readonly uncategorised = $localize`:@@settings.keyboard.uncategorised:General`;

  protected readonly query = signal('');
  /** The command whose key is being recorded. */
  protected readonly recording = signal<string | null>(null);
  protected readonly pending = signal<PendingConflict | null>(null);
  protected readonly status = signal('');

  protected readonly rows = computed(() => shortcutRows(this.registry.all(), (id) => this.registry.declaredKeybinding(id), this.query(), this.uncategorised));
  protected readonly hasOverrides = computed(() => Object.keys(this.store.overrides()).length > 0);

  constructor() {
    // While recording, every key press belongs to the editor: no command runs and the page does not react.
    effect((onCleanup) => {
      const id = this.recording();
      this.registry.keysSuspended.set(id !== null);
      if (id === null) return;
      const onKey = (e: KeyboardEvent) => this.onKey(e, id);
      this.doc.addEventListener('keydown', onKey, true);
      onCleanup(() => {
        this.doc.removeEventListener('keydown', onKey, true);
        this.registry.keysSuspended.set(false);
      });
    });
    // A conflict question takes focus (on the safe answer) so the keyboard user lands on it.
    effect(() => {
      if (this.pending()) queueMicrotask(() => this.cancelButton()?.nativeElement.focus());
    });
  }

  protected changeLabel(title: string, keys: string): string {
    return keys ? $localize`:@@settings.keyboard.change.withKey:Change the shortcut of ${title}:title:, now ${keys}:keys:` : $localize`:@@settings.keyboard.change.noKey:Set a shortcut for ${title}:title:, now None`;
  }

  protected resetLabel(title: string): string {
    return $localize`:@@settings.keyboard.reset.label:Reset the shortcut of ${title}:title: to its default`;
  }

  protected toggleRecording(id: string, title: string): void {
    this.pending.set(null);
    if (this.recording() === id) {
      this.recording.set(null);
      return;
    }
    this.recording.set(id);
    this.status.set($localize`:@@settings.keyboard.recording:Press the new shortcut for ${title}:title:. Esc cancels, Backspace removes the key.`);
  }

  /** Leaving the button ends the recording (unless a conflict question has taken over). */
  protected stopOnBlur(id: string): void {
    if (this.recording() === id && !this.pending()) {
      this.recording.set(null);
      this.status.set('');
    }
  }

  private onKey(e: KeyboardEvent, id: string): void {
    const row = this.rows().find((r) => r.id === id) ?? this.registry.all().find((c) => c.id === id);
    const title = row?.title ?? id;
    const rec = recordKey(e);
    // Tab and the like keep their meaning for moving focus; the blur ends the recording.
    if (rec.kind === 'invalid' && (e.key === 'Tab' || e.key === 'Enter' || e.key === ' ')) return;
    e.preventDefault();
    e.stopPropagation();
    switch (rec.kind) {
      case 'wait':
        return;
      case 'cancel':
        this.pending.set(null);
        this.recording.set(null);
        this.status.set($localize`:@@settings.keyboard.cancelled:Cancelled; ${title}:title: keeps its shortcut.`);
        return;
      case 'clear':
        this.commit(id, null);
        this.recording.set(null);
        this.status.set($localize`:@@settings.keyboard.cleared:${title}:title: has no shortcut now.`);
        return;
      case 'invalid':
        this.status.set($localize`:@@settings.keyboard.invalid:That key cannot be used on its own. Hold Ctrl, Alt or ⌘ with it, or use a function key.`);
        return;
      case 'binding': {
        const occupants = findConflicts(rec.binding, id, this.registry.all(), FIXED_KEYS);
        if (occupants.length > 0) {
          const p: PendingConflict = { id, title, binding: rec.binding, occupants };
          this.pending.set(p);
          this.status.set(this.conflictText(p));
          return;
        }
        this.commit(id, rec.binding);
        this.recording.set(null);
        this.status.set($localize`:@@settings.keyboard.set:${title}:title: is now ${formatKeybinding(rec.binding)}:keys:.`);
      }
    }
  }

  protected conflictText(p: PendingConflict): string {
    const keys = formatKeybinding(p.binding);
    const names = p.occupants.map((o) => (o.kind === 'command' ? `“${o.title}”` : `“${o.description}”`)).join(', ');
    return this.canReplace(p)
      ? $localize`:@@settings.keyboard.conflict:${keys}:keys: is already used by ${names}:names:. Replace it, so that only ${p.title}:title: uses ${keys}:keys:?`
      : $localize`:@@settings.keyboard.conflictFixed:${keys}:keys: is a built-in key (${names}:names:) and cannot be reassigned. Choose another key.`;
  }

  /** Commands can give a key up; fixed keys are handled inside components and cannot. */
  protected canReplace(p: PendingConflict): boolean {
    return p.occupants.every((o) => o.kind === 'command');
  }

  protected replace(p: PendingConflict): void {
    let next = this.store.overrides();
    for (const o of p.occupants) if (o.kind === 'command') next = withOverride(next, o.id, null, this.registry.declaredKeybinding(o.id));
    this.store.set(withOverride(next, p.id, p.binding, this.registry.declaredKeybinding(p.id)));
    this.finishConflict($localize`:@@settings.keyboard.replaced:${p.title}:title: is now ${formatKeybinding(p.binding)}:keys:.`);
  }

  protected cancelPending(): void {
    const p = this.pending();
    this.finishConflict($localize`:@@settings.keyboard.cancelled:Cancelled; ${p?.title ?? ''}:title: keeps its shortcut.`);
  }

  private finishConflict(message: string): void {
    const id = this.pending()?.id;
    this.pending.set(null);
    this.recording.set(null);
    this.status.set(message);
    if (id) queueMicrotask(() => this.doc.querySelector<HTMLElement>(`tr[data-command="${CSS.escape(id)}"] .key`)?.focus());
  }

  private commit(id: string, binding: string | null): void {
    this.store.set(withOverride(this.store.overrides(), id, binding, this.registry.declaredKeybinding(id)));
  }

  protected reset(id: string, title: string): void {
    this.store.set(withoutOverride(this.store.overrides(), id));
    this.status.set($localize`:@@settings.keyboard.resetDone:${title}:title: is back to its default shortcut.`);
  }

  protected resetAll(): void {
    this.store.resetAll();
    this.recording.set(null);
    this.pending.set(null);
    this.status.set($localize`:@@settings.keyboard.resetAllDone:All shortcuts are back to their defaults.`);
  }
}
