import { CdkMenu, CdkMenuItem, CdkMenuTrigger } from '@angular/cdk/menu';
import { ChangeDetectionStrategy, Component, computed, inject, OnInit } from '@angular/core';
import { ParserEntryDto, ParserErrorDto, ParserOrigin, SourceInfo } from '../../core/api.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { Switch } from '../../shared/switch';
import { DialogService } from '../../ui/dialog.service';
import { ParsersStore } from './parsers.store';
import { AiLauncher } from '../../ai/ai-launcher';

/** How many source names a parser row lists before "and N more". */
const NAMES_SHOWN = 3;

/**
 * Settings ▸ Parsers (ADR 0026): the library grouped by category, with each parser's version, origin, kind, enabled
 * switch and the files of this window it read; edit (TOML with validation and a live preview), duplicate, export,
 * remove; add from a file (the format wizard), from a .toml file or blank; broken user files; and "Re-read" when
 * sources were read with parsers that changed since.
 */
@Component({
  selector: 'imt-parsers-settings',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Switch, CdkMenu, CdkMenuItem, CdkMenuTrigger],
  template: `
    <div class="section-head">
      <h3 i18n="Settings section|The log formats IMT reads@@settings.parsers.title">Parsers</h3>
      <span class="spacer"></span>
      <button type="button" class="btn" data-testid="parsers-new-from-file" [disabled]="busy() || !sources().length" [cdkMenuTriggerFor]="fileMenu" i18n="@@settings.parsers.newFromFile">New from a file…</button>
      <button type="button" class="btn" data-testid="parsers-generate-ai" [disabled]="busy() || !sources().length" [cdkMenuTriggerFor]="aiMenu" i18n="@@settings.parsers.generateAi">Generate with AI…</button>
      <button type="button" class="btn" data-testid="parsers-import" [disabled]="busy()" (click)="store.importFromFile()" i18n="@@settings.parsers.import">Import .toml…</button>
      <button type="button" class="btn" data-testid="parsers-new-blank" [disabled]="busy()" (click)="store.newBlank()" i18n="@@settings.parsers.newBlank">New blank</button>
    </div>
    <p class="desc" i18n="@@settings.parsers.desc">A parser describes one log format and is one TOML file you can share. For every file, IMT uses the enabled parser that reads it best; “Parse with…” in the Explorer chooses one yourself.</p>

    @if (store.stale(); as n) {
      <div class="banner warn" role="status" data-testid="parsers-stale">
        <span i18n="@@settings.parsers.stale">{n, plural, =1 {1 source was read with a parser that changed.} other {{{ n }} sources were read with parsers that changed.}}</span>
        <button type="button" class="btn primary" [disabled]="busy()" (click)="store.reread()" i18n="@@settings.parsers.reread">Re-read</button>
      </div>
    }
    @if (store.notice(); as n) {
      <div class="banner ok" role="status" data-testid="parsers-notice"><span>{{ n }}</span><button type="button" class="btn ghost" (click)="store.dismiss()" i18n="@@settings.parsers.dismiss">Dismiss</button></div>
    }
    @if (store.actionError(); as e) {
      <div class="banner err" role="alert"><span>{{ e }}</span><button type="button" class="btn ghost" (click)="store.dismiss()" i18n="@@settings.parsers.dismiss">Dismiss</button></div>
    }

    @if (store.loadError(); as e) {
      <div class="banner err" role="alert">
        <span i18n="@@settings.parsers.loadError">Could not list the parsers: {{ e }}</span>
        <button type="button" class="btn" (click)="store.load()" i18n="@@settings.parsers.retry">Retry</button>
      </div>
    } @else if (!store.library()) {
      <p class="muted" i18n="@@settings.parsers.loading">Loading parsers…</p>
    }

    @for (g of store.groups(); track g.category) {
      <h4>{{ g.category }}</h4>
      <ul class="list" [attr.aria-label]="g.category">
        @for (p of g.parsers; track p.id) {
          <li class="parser" [class.off]="!p.enabled" [attr.data-parser]="p.id">
            <div class="main">
              <div class="title">
                <span class="name">{{ p.name }}</span>
                <span class="mono faint">{{ p.id }}</span>
                <span class="mono faint">{{ p.version }}</span>
                <span class="badge" [attr.data-origin]="p.origin">{{ originLabel(p.origin) }}</span>
                <span class="kind mono">{{ p.kind }}</span>
              </div>
              <div class="read" [attr.data-testid]="'parser-read-' + p.id">{{ readText(p) }}</div>
            </div>
            <div class="actions">
              <imt-switch size="small" [checked]="p.enabled" [disabled]="busy()" (checkedChange)="store.setEnabled(p.id, $event)"><span class="sr-only">{{ enableLabel(p) }}</span><span aria-hidden="true" i18n="@@settings.parsers.enabled">Enabled</span></imt-switch>
              <button type="button" class="btn ghost small" [disabled]="busy()" [attr.aria-label]="actionLabel(editText, p)" (click)="store.edit(p.id)">{{ editText }}</button>
              <button type="button" class="btn ghost small" [disabled]="busy()" [attr.aria-label]="actionLabel(duplicateText, p)" (click)="store.duplicate(p.id)">{{ duplicateText }}</button>
              <button type="button" class="btn ghost small" [disabled]="busy()" [attr.aria-label]="actionLabel(exportText, p)" (click)="store.exportParser(p.id)">{{ exportText }}</button>
              @if (p.origin !== 'builtin') {
                <button type="button" class="btn ghost small danger" [disabled]="busy()" [attr.aria-label]="actionLabel(removeText, p)" (click)="store.remove(p.id)">{{ removeText }}</button>
              }
            </div>
          </li>
        }
      </ul>
    }

    @if (store.broken().length) {
      <h4 i18n="@@settings.parsers.brokenTitle">Parser files that could not be read</h4>
      <ul class="list" aria-label="Broken parser files" i18n-aria-label="@@settings.parsers.brokenLabel">
        @for (b of store.broken(); track b.file) {
          <li class="parser broken" role="listitem">
            <span class="mono">{{ b.file }}</span>
            <span class="err">{{ errorText(b.error) }}</span>
          </li>
        }
      </ul>
      <p class="muted" i18n="@@settings.parsers.brokenHint">Broken files are never used. Fix them in a text editor, or import a corrected copy.</p>
    }

    <ng-template #aiMenu>
      <div cdkMenu class="menu" aria-label="Generate a parser for a file of this window" i18n-aria-label="@@settings.parsers.aiMenu">
        @for (s of sources(); track s.id) {
          <button type="button" cdkMenuItem class="mi" (cdkMenuItemTriggered)="generateWithAi(s)">{{ s.name }}</button>
        }
      </div>
    </ng-template>

    <ng-template #fileMenu>
      <div cdkMenu class="menu" aria-label="Files of this window" i18n-aria-label="@@settings.parsers.fileMenu">
        @for (s of sources(); track s.id) {
          <button type="button" cdkMenuItem class="mi" (cdkMenuItemTriggered)="newFromFile(s)">{{ s.name }}</button>
        }
      </div>
    </ng-template>
  `,
  styles: `
    :host { display: block; }
    .section-head { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
    h3 { margin: 0 0 4px; font-size: var(--fs-lg); font-weight: 600; }
    h4 { margin: 14px 0 4px; font-size: var(--fs-xs); font-weight: 600; letter-spacing: 0.08em; text-transform: uppercase; color: var(--muted); }
    .spacer { flex: 1; }
    .desc { margin: 4px 0 8px; color: var(--muted); line-height: 1.45; }
    .muted { color: var(--muted); }
    .faint { color: var(--faint); font-size: var(--fs-xs); }
    .banner { display: flex; align-items: center; gap: 10px; margin: 6px 0; padding: 8px 10px; border-radius: 6px; border: 1px solid var(--border-3); background: var(--bg-3); color: var(--text-2); }
    .banner > span { flex: 1; }
    .banner.warn { border-color: var(--warn-text); }
    .banner.ok { border-color: var(--ok); }
    .banner.err { border-color: var(--danger-text); color: var(--danger-text); }
    .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 4px; }
    .parser { display: flex; align-items: center; gap: 10px; padding: 6px 8px; border: 1px solid var(--border); border-radius: 6px; background: var(--bg-2); flex-wrap: wrap; }
    .parser.off .name { color: var(--muted); }
    .parser.broken { flex-direction: column; align-items: flex-start; gap: 2px; }
    .main { flex: 1; min-width: 220px; display: flex; flex-direction: column; gap: 2px; }
    .title { display: flex; align-items: baseline; gap: 8px; flex-wrap: wrap; }
    .name { font-weight: 600; }
    .badge { font-size: var(--fs-2xs); padding: 0 6px; border-radius: 8px; border: 1px solid var(--border-3); color: var(--text-2); }
    .badge[data-origin='user'] { border-color: var(--accent); }
    .badge[data-origin='override'] { border-color: var(--warn-text); }
    .kind { font-size: var(--fs-xs); color: var(--muted); }
    .read { font-size: var(--fs-sm); color: var(--text-2); overflow-wrap: anywhere; }
    .actions { display: flex; align-items: center; gap: 4px; flex-wrap: wrap; }
    .btn.danger { color: var(--danger-text); }
    .err { color: var(--danger-text); font-size: var(--fs-sm); }
    .menu { min-width: 220px; max-width: min(360px, calc(100vw - 16px)); max-height: 320px; overflow-y: auto; display: flex; flex-direction: column; padding: 4px; border-radius: 8px; background: var(--bg-2); border: 1px solid var(--border-3); box-shadow: var(--shadow); }
    .menu:focus { outline: none; }
    .mi { display: flex; align-items: center; min-height: 28px; padding: 0 10px; border: 0; border-radius: 5px; background: transparent; color: var(--text-2); cursor: pointer; text-align: start; font: inherit; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .mi:hover, .mi:focus { background: var(--bg-active); color: var(--text); outline: none; }
    .mi:focus-visible { box-shadow: inset 0 0 0 1px var(--accent); }
  `,
})
export class ParsersSettings implements OnInit {
  protected readonly store = inject(ParsersStore);
  private readonly workspace = inject(WorkspaceStore);
  private readonly dialogs = inject(DialogService);
  private readonly ai = inject(AiLauncher);

  protected readonly sources = computed<SourceInfo[]>(() => this.workspace.summary()?.sources ?? []);
  protected readonly busy = computed(() => this.store.busy() !== null);
  protected readonly editText = $localize`:@@settings.parsers.edit:Edit`;
  protected readonly duplicateText = $localize`:@@settings.parsers.duplicate:Duplicate`;
  protected readonly exportText = $localize`:@@settings.parsers.export:Export…`;
  protected readonly removeText = $localize`:@@settings.parsers.remove:Remove`;

  ngOnInit(): void {
    void this.store.load();
  }

  protected originLabel(origin: ParserOrigin): string {
    switch (origin) {
      case 'builtin':
        return $localize`:Parser origin badge@@settings.parsers.origin.builtin:Built-in`;
      case 'user':
        return $localize`:Parser origin badge@@settings.parsers.origin.user:Custom`;
      case 'override':
        return $localize`:Parser origin badge@@settings.parsers.origin.override:Overrides built-in`;
    }
  }

  /** "Read 2 files: a.log, b.log" (the sources of this window), or that it read none. */
  protected readText(p: ParserEntryDto): string {
    const n = p.sources.length;
    if (!n) return $localize`:@@settings.parsers.readNone:Read no files in this window`;
    const names = p.sources.slice(0, NAMES_SHOWN).map((s) => s.name).join(', ');
    const more = n - NAMES_SHOWN;
    const list = more > 0 ? $localize`:@@settings.parsers.readMore:${names}:names: and ${more}:more: more` : names;
    return n === 1 ? $localize`:@@settings.parsers.readOne:Read 1 file: ${list}:list:` : $localize`:@@settings.parsers.readMany:Read ${n}:count: files: ${list}:list:`;
  }

  protected actionLabel(action: string, p: ParserEntryDto): string {
    const name = p.name;
    return $localize`:Button label, e.g. "Edit Serilog CLEF"@@settings.parsers.actionLabel:${action}:action: ${name}:name:`;
  }

  protected enableLabel(p: ParserEntryDto): string {
    const name = p.name;
    return $localize`:@@settings.parsers.enableLabel:Use ${name}:name:`;
  }

  protected errorText(e: ParserErrorDto): string {
    const message = e.message;
    if (e.line === null) return message;
    const line = e.line;
    const column = e.column ?? 1;
    return $localize`:@@settings.parsers.errorAt:Line ${line}:line:, column ${column}:column:: ${message}:message:`;
  }

  /** "New from a file…": the format wizard teaches a new parser from that file. */
  /** The local model drafts a parser for the file; the engine checks every draft on it (ADR 0032). */
  protected async generateWithAi(s: SourceInfo): Promise<void> {
    await this.ai.propose('parser', { sourceId: s.id, fileName: s.name });
  }

  protected async newFromFile(s: SourceInfo): Promise<void> {
    const { FormatWizard } = await import('../../format/format-wizard');
    this.dialogs.open(FormatWizard, { title: $localize`:@@settings.parsers.wizardTitle:New parser from ${s.name}:name:`, inputs: { path: s.path, name: s.name }, width: 760 });
  }
}
