import { ChangeDetectionStrategy, Component, EnvironmentProviders, inject, Injectable, input, makeEnvironmentProviders, signal } from '@angular/core';
import { AiLauncher } from '../ai/ai-launcher';
import { DEBRIEF_RECORD_MENU_ITEMS, RecordMenuContribution } from '../context/record-actions';
import { RecordView } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { DEBRIEF_COMMANDS, DEBRIEF_INSPECTOR_SECTIONS, DebriefCommand, DebriefInspectorSection } from '../extensions/tokens';
import { DialogService } from '../ui/dialog.service';

/** "Search the web for this error" from anywhere (menus, inspector, chat, palette). */
@Injectable({ providedIn: 'root' })
export class BrowserLauncher {
  private readonly dialogs = inject(DialogService);

  /**
   * The AI drafts the search text from the event (the engine without a model); the user edits it, then the search
   * opens in their default browser.
   */
  async searchEvent(recordId: number): Promise<void> {
    const { SearchWebDialog } = await import('./search-web-dialog');
    const ref = this.dialogs.open(SearchWebDialog, {
      title: $localize`:Dialog title@@browser.searchDialog.title:Search the web for this error`,
      width: 560,
      inputs: { recordId },
    });
    await ref.closed;
  }
}

/** Errors and warnings, and anything with an exception, are worth looking up. */
export function worthSearching(r: RecordView): boolean {
  return !!r.exception || r.level === 'Warning' || r.level === 'Error' || r.level === 'Fatal';
}

/** The inspector's Web section: one button for the selected event. */
@Component({
  selector: 'debrief-web-inspector-section',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<div class="pad">
    <button type="button" class="btn" (click)="launcher.searchEvent(record().id)" data-testid="inspector-search-web" i18n="@@browser.inspector.search">Search the web for this error…</button>
    <span class="research">
      <button type="button" class="btn primary" (click)="research()" data-testid="inspector-research" title="The AI searches the web, picks and reads the results (read-only) and answers in the chat with citations" i18n-title="@@browser.inspector.researchTitle" i18n="@@browser.inspector.research">Research this</button>
      <label class="sr-only" for="inspector-research-results" i18n="@@browser.inspector.resultsLabel">Results to read</label>
      <select id="inspector-research-results" class="select" [value]="results()" (change)="results.set(asValue($event))" data-testid="inspector-research-results">
        <option value="" i18n="@@browser.inspector.resultsDefault">Default</option>
        <option value="10" i18n="@@browser.inspector.results10">10 results</option>
        <option value="20" i18n="@@browser.inspector.results20">20 results</option>
        <option value="30" i18n="@@browser.inspector.results30">30 results</option>
      </select>
    </span>
    @if (note(); as n) { <p class="err small" role="alert">{{ n }}</p> }
  </div>`,
  styles: `.pad { padding: 0 14px 6px; display: flex; flex-wrap: wrap; gap: 6px; align-items: center; } .research { display: inline-flex; gap: 4px; } .err { margin: 0; color: var(--danger-text, var(--lvl-error)); } .small { font-size: var(--fs-xs); } .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); }`,
})
export class WebInspectorSection {
  protected readonly launcher = inject(BrowserLauncher);
  private readonly ai = inject(AiLauncher);
  readonly record = input.required<RecordView>();
  /** Results to read ('' = the setting). */
  protected readonly results = signal('');
  protected readonly note = signal<string | null>(null);

  protected asValue(e: Event): string {
    return (e.target as HTMLSelectElement).value;
  }

  protected async research(): Promise<void> {
    const n = this.results();
    this.note.set(await this.ai.research({ recordId: this.record().id, maxResults: n ? Number(n) : null }));
  }
}

export const WEB_INSPECTOR_SECTION: DebriefInspectorSection = {
  id: 'web',
  title: $localize`:Inspector section@@browser.inspector.title:Web`,
  order: 450,
  component: WebInspectorSection,
  when: worthSearching,
};

export function browserCommands(launcher: BrowserLauncher, store: InstanceType<typeof WorkspaceStore>): DebriefCommand[] {
  const category = $localize`:Command palette category@@browser.category:Web`;
  return [
    {
      id: 'browser.searchSelected',
      title: $localize`:@@browser.cmd.searchSelected:Web: Search the web for the selected event…`,
      category,
      when: () => !!store.selected(),
      run: () => {
        const r = store.selected();
        if (r) void launcher.searchEvent(r.id);
      },
    },
  ];
}

/** "Search the web for this error…" in the grid and inspector context menu. */
export function browserRecordMenu(launcher: BrowserLauncher, ai?: AiLauncher): RecordMenuContribution {
  return (record) => [
    { label: $localize`:@@browser.menu.search:Search the web for this error…`, run: () => void launcher.searchEvent(record.id) },
    ...(ai ? [{ label: $localize`:@@browser.menu.research:Research this error`, run: () => void ai.research({ recordId: record.id }) }] : []),
  ];
}

/** One line in `app.config.ts`: the browser's commands, menu entries and inspector section. */
export function provideBrowser(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: DEBRIEF_COMMANDS, multi: true, useFactory: () => browserCommands(inject(BrowserLauncher), inject(WorkspaceStore)) },
    { provide: DEBRIEF_RECORD_MENU_ITEMS, multi: true, useFactory: () => browserRecordMenu(inject(BrowserLauncher), inject(AiLauncher)) },
    { provide: DEBRIEF_INSPECTOR_SECTIONS, multi: true, useValue: [WEB_INSPECTOR_SECTION] },
  ]);
}
