import { ChangeDetectionStrategy, Component, EnvironmentProviders, inject, Injectable, input, makeEnvironmentProviders, signal } from '@angular/core';
import { AiLauncher } from '../ai/ai-launcher';
import { IMT_RECORD_MENU_ITEMS, RecordMenuContribution } from '../context/record-actions';
import { RecordView } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { DockService } from '../dock/dock.service';
import { IMT_COMMANDS, IMT_INSPECTOR_SECTIONS, IMT_VIEWS, ImtCommand, ImtInspectorSection, ImtView } from '../extensions/tokens';
import { DialogService } from '../ui/dialog.service';

export const WEB_VIEW = 'web';

/** Opens the Web panel and "Search the web for this error" from anywhere (menus, inspector, chat, palette). */
@Injectable({ providedIn: 'root' })
export class BrowserLauncher {
  private readonly dialogs = inject(DialogService);
  private readonly dock = inject(DockService);

  showPanel(): void {
    this.dock.show(WEB_VIEW);
  }

  /** The engine builds the search text from the event; the user edits it, then the browser opens. */
  async searchEvent(recordId: number): Promise<void> {
    const { SearchWebDialog } = await import('./search-web-dialog');
    const ref = this.dialogs.open<boolean>(SearchWebDialog, {
      title: $localize`:Dialog title@@browser.searchDialog.title:Search the web for this error`,
      width: 560,
      inputs: { recordId },
    });
    if (await ref.closed) this.showPanel();
  }
}

/** Errors and warnings, and anything with an exception, are worth looking up. */
export function worthSearching(r: RecordView): boolean {
  return !!r.exception || r.level === 'Warning' || r.level === 'Error' || r.level === 'Fatal';
}

/** The inspector's Web section: one button for the selected event. */
@Component({
  selector: 'imt-web-inspector-section',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<div class="pad">
    <button type="button" class="btn" (click)="launcher.searchEvent(record().id)" data-testid="inspector-search-web" i18n="@@browser.inspector.search">Search the web for this error…</button>
    <span class="research">
      <button type="button" class="btn primary" (click)="research()" data-testid="inspector-research" title="The AI searches the web, reads the top results (read-only) and answers in the chat with citations" i18n-title="@@browser.inspector.researchTitle" i18n="@@browser.inspector.research">Research this</button>
      <label class="sr-only" for="inspector-research-results" i18n="@@browser.inspector.resultsLabel">Results to read</label>
      <select id="inspector-research-results" class="select" [value]="results()" (change)="results.set(asValue($event))" data-testid="inspector-research-results">
        <option value="" i18n="@@browser.inspector.resultsDefault">Default</option>
        <option value="3" i18n="@@browser.inspector.results3">3 results</option>
        <option value="5" i18n="@@browser.inspector.results5">5 results</option>
        <option value="10" i18n="@@browser.inspector.results10">10 results</option>
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

export const WEB_INSPECTOR_SECTION: ImtInspectorSection = {
  id: 'web',
  title: $localize`:Inspector section@@browser.inspector.title:Web`,
  order: 450,
  component: WebInspectorSection,
  when: worthSearching,
};

export function browserViews(): ImtView[] {
  return [
    {
      id: WEB_VIEW,
      title: $localize`:View title@@browser.view.title:Web`,
      icon: 'globe',
      category: 'Tools',
      defaultPosition: 'right',
      component: () => import('./web-panel').then((m) => m.WebPanel),
    },
  ];
}

export function browserCommands(launcher: BrowserLauncher, store: InstanceType<typeof WorkspaceStore>): ImtCommand[] {
  const category = $localize`:Command palette category@@browser.category:Web`;
  return [
    { id: 'browser.show', title: $localize`:@@browser.cmd.show:Web: Open the browser panel`, category, run: () => launcher.showPanel() },
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

/** One line in `app.config.ts`: the Web view (Tools ▸ and View ▸ Tools), its commands and menu entries. */
export function provideBrowser(): EnvironmentProviders {
  return makeEnvironmentProviders([
    { provide: IMT_VIEWS, multi: true, useFactory: browserViews },
    { provide: IMT_COMMANDS, multi: true, useFactory: () => browserCommands(inject(BrowserLauncher), inject(WorkspaceStore)) },
    { provide: IMT_RECORD_MENU_ITEMS, multi: true, useFactory: () => browserRecordMenu(inject(BrowserLauncher), inject(AiLauncher)) },
    { provide: IMT_INSPECTOR_SECTIONS, multi: true, useValue: [WEB_INSPECTOR_SECTION] },
  ]);
}
