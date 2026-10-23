import { CdkListbox, CdkOption } from '@angular/cdk/listbox';
import { ChangeDetectionStrategy, Component, computed, effect, inject, linkedSignal, signal, untracked } from '@angular/core';
import { symbolProblemLines, symbolProblemTotal, symbolStatusHint, symbolStatusLabel } from './symbol-status';
import { DumpFinding, DumpOverview, FrameTrust, ModulesPage, StackDto, ThreadsPage, ThreadState } from '../core/api.types';
import { formatDateTime, formatDuration } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { AnalysisState } from '../analysis/analysis-state';
import { AsyncResource, errorText } from '../analysis/async-resource';
import { headlineText } from './dump-headline';
import { DumpsBackend } from './dumps-backend';
import { DumpFindings, DumpsExplainer, DUMPS_STYLES, formatBytes, formatCount, NoDump } from './dumps-shared';
import { DumpsStore } from './dumps.store';

export type DumpTab = 'findings' | 'threads' | 'modules' | 'details';

/** Rows asked for at once (the engine pages; a dump has at most 4,096 threads and modules). */
const PAGE = 500;

const STATE_LABEL: Record<ThreadState, string> = {
  faulted: $localize`:@@dumps.thread.faulted:faulted`,
  lockWait: $localize`:@@dumps.thread.lockWait:waits on a lock`,
  waiting: $localize`:@@dumps.thread.waiting:waiting`,
  running: $localize`:@@dumps.thread.running:running`,
};

const TRUST_LABEL: Record<FrameTrust, string> = {
  context: $localize`:@@dumps.trust.context:context`,
  cfi: $localize`:@@dumps.trust.cfi:unwind info`,
  framePointer: $localize`:@@dumps.trust.framePointer:frame pointer`,
  scan: $localize`:@@dumps.trust.scan:stack scan`,
  recorded: $localize`:@@dumps.trust.recorded:recorded`,
};

/**
 * A crash or memory dump, problem first: the exception card and the engine's findings (crash cause, culprit,
 * lock waits), then threads with their stacks, modules and details. Everything is computed by the engine.
 */
@Component({
  selector: 'debrief-dump-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AnalysisState, CdkListbox, CdkOption, DumpFindings, NoDump],
  template: `
    @if (store.dump(); as doc) {
      <div class="bar">
        <label class="inline">
          <span class="sr" i18n="@@dumps.view.pick">Dump</span>
          <select data-testid="dump-pick" [value]="doc.id" (change)="pick($event)" aria-label="Dump" i18n-aria-label="@@dumps.view.pickLabel">
            @for (d of store.dumps(); track d.id) {
              <option [value]="d.id">{{ d.name }}</option>
            }
          </select>
        </label>
        <span class="ellipsis grow" data-testid="dump-headline">{{ headline(doc.headline) }}</span>
        <button class="btn ghost" type="button" (click)="store.close(doc.id)" i18n="@@dumps.view.close">Close</button>
      </div>
      @if (store.error(); as e) {
        <div class="notice err" role="alert">{{ e }}</div>
      }
      <debrief-analysis-state [loading]="overview.loading()" [error]="overview.error()" [retryable]="overview.retryable()" [hasData]="!!overview.data()" (retry)="overview.retry()">
        @if (overview.data(); as o) {
          <div class="summary">
            <span>{{ o.processName ?? doc.name }}@if (o.pid !== null) { <span class="faint"> · pid {{ o.pid }}</span> }</span>
            @if (o.os) { <span class="faint">{{ o.os }}</span> }
            @if (o.arch) { <span class="tag">{{ o.arch }}</span> }
            <span class="tag">{{ kindLabel(o.dumpKind) }}</span>
            @if (doc.timeMs !== null) { <span class="faint">{{ when(doc.timeMs) }}</span> }
            @if (o.uptimeS !== null) { <span class="faint" i18n="@@dumps.view.uptime">up {{ uptime(o.uptimeS) }}</span> }
            @if (o.dotnet) { <span class="tag">{{ o.dotnet }}</span> }
          </div>
          @if (o.exception; as x) {
            <section class="card exception" aria-label="Exception" i18n-aria-label="@@dumps.view.exception" data-testid="dump-exception">
              <div class="head"><strong>{{ x.name }}</strong> <span class="mono faint">{{ x.code }}</span>@if (x.access) { <span class="mono">{{ x.access }}</span> }</div>
              @if (x.meaning) { <p class="detail">{{ x.meaning }}</p> }
              @if (x.fastFail) { <p class="detail mono">{{ x.fastFail }}</p> }
              <div class="faint">
                <span i18n="@@dumps.view.at">at</span> <span class="mono">{{ x.location ?? x.address }}</span> ·
                <button class="link" type="button" (click)="showThread(x.threadId, 0)" i18n="@@dumps.view.thread">thread {{ x.threadId }}</button>
              </div>
            </section>
          }
        }
      </debrief-analysis-state>
      <div class="tabs" role="tablist" aria-label="Dump views" i18n-aria-label="@@dumps.view.tabs">
        @for (t of tabs; track t.id) {
          <button type="button" role="tab" [id]="'dump-tab-' + t.id" [attr.aria-selected]="tab() === t.id" [attr.aria-controls]="'dump-panel-' + t.id" [attr.data-testid]="'dump-tab-' + t.id" (click)="tab.set(t.id)">{{ t.label }}</button>
        }
      </div>
      <div class="panel" role="tabpanel" [id]="'dump-panel-' + tab()" [attr.aria-labelledby]="'dump-tab-' + tab()">
        @switch (tab()) {
          @case ('findings') {
            <debrief-dump-findings [res]="findings" (evidence)="showEvidence($event)" (explain)="explain()" />
          }
          @case ('threads') {
            <div class="split">
              <div class="side">
                <div class="bar"><input type="search" [value]="threadFilter()" (input)="threadFilter.set(value($event))" placeholder="Filter threads" i18n-placeholder="@@dumps.threads.filter" aria-label="Filter threads" i18n-aria-label="@@dumps.threads.filterLabel" /></div>
                <debrief-analysis-state [loading]="threads.loading()" [error]="threads.error()" [retryable]="threads.retryable()" [hasData]="!!threads.data()" [empty]="threads.data()?.total === 0" emptyText="No threads." i18n-emptyText="@@dumps.threads.empty" (retry)="threads.retry()">
                  <ul cdkListbox class="threads" aria-label="Threads" i18n-aria-label="@@dumps.threads.label" data-testid="dump-threads" [cdkListboxValue]="threadValue()" (cdkListboxValueChange)="selectThread($event.value[0])">
                    @for (t of threads.data()?.rows ?? []; track t.id) {
                      <li [cdkOption]="t.id" class="thread" [class.faulted]="t.state === 'faulted'" [attr.data-state]="t.state">
                        <span class="mono">{{ t.id }}</span>
                        @if (t.name) { <span class="ellipsis">{{ t.name }}</span> }
                        <span class="tag">{{ stateLabel(t.state) }}</span>
                        <span class="ellipsis faint mono">{{ t.userFrame ?? t.top }}</span>
                      </li>
                    }
                  </ul>
                </debrief-analysis-state>
              </div>
              <div class="main" tabindex="0" role="region" aria-label="Stack" i18n-aria-label="@@dumps.stack.region">
                <debrief-analysis-state [loading]="stack.loading()" [error]="stack.error()" [retryable]="stack.retryable()" [hasData]="!!stack.data()" [empty]="stack.data()?.frames?.length === 0" emptyText="No frames: the thread's stack could not be read." i18n-emptyText="@@dumps.stack.empty" (retry)="stack.retry()">
                  @if (stack.data(); as s) {
                    <table class="grid" data-testid="dump-stack" [attr.aria-label]="stackLabel(s.threadId)">
                      <thead><tr><th class="num">#</th><th i18n="@@dumps.stack.frame">Frame</th><th i18n="@@dumps.stack.source">Source</th><th i18n="@@dumps.stack.how">Found by</th></tr></thead>
                      <tbody>
                        @for (f of s.frames; track f.index) {
                          <tr [class.framework]="f.framework" [class.culprit]="f.culprit" [class.sel]="f.index === highlightFrame()" [attr.data-culprit]="f.culprit || null">
                            <td class="num">{{ f.index }}</td>
                            <td class="mono">{{ f.label }}@if (f.culprit) { <span class="tag culprit" i18n="@@dumps.stack.culprit">likely culprit</span> }</td>
                            <td class="mono faint">{{ f.source ?? '' }}</td>
                            <td><span class="tag">{{ trustLabel(f.trust) }}</span></td>
                          </tr>
                        }
                      </tbody>
                    </table>
                    @if (s.truncated) { <p class="faint" i18n="@@dumps.stack.truncated">The stack is deeper; only its innermost frames are shown.</p> }
                  }
                </debrief-analysis-state>
              </div>
            </div>
          }
          @case ('modules') {
            <div class="bar"><input type="search" [value]="moduleFilter()" (input)="moduleFilter.set(value($event))" placeholder="Filter modules" i18n-placeholder="@@dumps.modules.filter" aria-label="Filter modules" i18n-aria-label="@@dumps.modules.filterLabel" /></div>
            <debrief-analysis-state [loading]="modules.loading()" [error]="modules.error()" [retryable]="modules.retryable()" [hasData]="!!modules.data()" [empty]="modules.data()?.total === 0" emptyText="No modules." i18n-emptyText="@@dumps.modules.empty" (retry)="modules.retry()">
              <div class="body">
                <table class="grid" data-testid="dump-modules" aria-label="Modules" i18n-aria-label="@@dumps.modules.label">
                  <thead><tr><th i18n="@@dumps.modules.name">Module</th><th i18n="@@dumps.modules.version">Version</th><th i18n="@@dumps.modules.base">Base</th><th class="num" i18n="@@dumps.modules.size">Size</th><th i18n="@@dumps.modules.symbols">Symbols</th><th i18n="@@dumps.modules.path">Path</th></tr></thead>
                  <tbody>
                    @for (m of modules.data()?.rows ?? []; track m.base) {
                      <tr [class.framework]="m.framework">
                        <td>{{ m.name }}</td>
                        <td class="mono">{{ m.version ?? '' }}</td>
                        <td class="mono">{{ m.base }}</td>
                        <td class="num">{{ bytes(m.size) }}</td>
                        <td [title]="hint(m.symbols) ?? ''" data-testid="module-symbols"><span [class.faint]="m.symbols.kind !== 'loaded'">{{ symbolLabel(m.symbols) }}</span></td>
                        <td class="ellipsis faint" [title]="m.path">{{ m.path }}</td>
                      </tr>
                    }
                  </tbody>
                </table>
              </div>
            </debrief-analysis-state>
          }
          @case ('details') {
            @if (overview.data(); as o) {
              <div class="body cards">
                <section class="card">
                  <h3 class="sec" i18n="@@dumps.details.symbols">Symbols</h3>
                  <p i18n="@@dumps.details.symbolsText">{{ count(o.framesSymbolised) }} of {{ count(o.frames) }} frames named; {{ count(o.modulesWithSymbols) }} of {{ count(o.modules) }} modules with symbols.</p>
                  @if (problemTotal(o.symbolProblems) > 0) {
                    <ul class="list" data-testid="symbol-problems">@for (l of problemLines(o.symbolProblems); track l) { <li>{{ l }}</li> }</ul>
                  }
                </section>
                @if (o.handles; as h) {
                  <section class="card">
                    <h3 class="sec" i18n="@@dumps.details.handles">Handles ({{ count(h.total) }})</h3>
                    <ul class="list">@for (t of h.byType; track t.name) { <li><span class="mono">{{ count(t.count) }}</span> {{ t.name }}</li> }</ul>
                  </section>
                }
                @if (o.memory; as m) {
                  <section class="card">
                    <h3 class="sec" i18n="@@dumps.details.memory">Memory</h3>
                    <ul class="list">
                      <li i18n="@@dumps.details.committed">Committed {{ bytes(m.committed) }} (private {{ bytes(m.private) }}, image {{ bytes(m.image) }}, mapped {{ bytes(m.mapped) }})</li>
                      <li i18n="@@dumps.details.reserved">Reserved {{ bytes(m.reserved) }}</li>
                      <li i18n="@@dumps.details.free">Largest free block {{ bytes(m.largestFree) }}</li>
                    </ul>
                  </section>
                }
                @if (o.unloadedModules.length) {
                  <section class="card">
                    <h3 class="sec" i18n="@@dumps.details.unloaded">Unloaded modules</h3>
                    <ul class="list">@for (u of o.unloadedModules; track $index) { <li>{{ u }}</li> }</ul>
                  </section>
                }
                @if (o.notes.length) {
                  <section class="card">
                    <h3 class="sec" i18n="@@dumps.details.notes">Notes</h3>
                    <ul class="list">@for (n of o.notes; track $index) { <li>{{ n }}</li> }</ul>
                  </section>
                }
              </div>
            }
          }
        }
      </div>
    } @else {
      <debrief-no-dump text="Open a crash or memory dump (.dmp) to see why the process crashed or hung." i18n-text="@@dumps.view.none" action="Open dump…" i18n-action="@@dumps.view.open" (open)="store.pickAndOpen('dump')" />
    }
  `,
  styles: [
    DUMPS_STYLES,
    `
      .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
      .summary { display: flex; flex-wrap: wrap; gap: 6px 12px; align-items: center; padding: 6px 12px; }
      .exception { margin: 0 12px 8px; }
      .exception .head { display: flex; gap: 8px; align-items: baseline; color: var(--text); }
      .exception .detail { margin: 4px 0; }
      .panel { flex: 1; min-height: 0; display: flex; flex-direction: column; }
      .split { flex: 1; min-height: 0; display: flex; }
      .side { width: 40%; min-width: 220px; border-right: 1px solid var(--border); display: flex; flex-direction: column; min-height: 0; }
      .main { flex: 1; min-width: 0; overflow: auto; padding: 8px 12px; }
      .threads { list-style: none; margin: 0; padding: 0; overflow: auto; flex: 1; }
      .thread { display: flex; gap: 8px; align-items: center; padding: 4px 12px; border-bottom: 1px solid var(--border); cursor: pointer; }
      .thread[aria-selected='true'] { background: var(--bg-sel); }
      .thread:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
      .thread.faulted .mono:first-child { color: var(--danger-text); }
    `,
  ],
})
export class DumpView {
  protected readonly store = inject(DumpsStore);
  private readonly backend = inject(DumpsBackend);
  private readonly explainer = inject(DumpsExplainer);
  private readonly workspace = inject(WorkspaceStore);

  protected readonly tabs: { id: DumpTab; label: string }[] = [
    { id: 'findings', label: $localize`:@@dumps.tab.findings:Findings` },
    { id: 'threads', label: $localize`:@@dumps.tab.threads:Threads` },
    { id: 'modules', label: $localize`:@@dumps.tab.modules:Modules` },
    { id: 'details', label: $localize`:@@dumps.tab.details:Details` },
  ];
  /** Problem first: a dump opens on its findings. */
  // Another dump starts from the findings, with no thread or frame picked.
  readonly tab = linkedSignal<number | null, DumpTab>({ source: () => this.id(), computation: () => 'findings' });
  readonly selectedThread = linkedSignal<number | null, number | null>({ source: () => this.id(), computation: () => null });
  readonly highlightFrame = linkedSignal<number | null, number | null>({ source: () => this.id(), computation: () => null });
  readonly threadFilter = signal('');
  readonly moduleFilter = signal('');

  readonly overview = new AsyncResource<DumpOverview>();
  readonly findings = new AsyncResource<DumpFinding[]>();
  readonly threads = new AsyncResource<ThreadsPage>();
  readonly stack = new AsyncResource<StackDto>();
  readonly modules = new AsyncResource<ModulesPage>();

  private readonly id = computed(() => this.store.dump()?.id ?? null);
  /** The listbox's value: the selected thread when it is listed (a listbox refuses values it has no option for). */
  protected readonly threadValue = computed(() => {
    const id = this.selectedThread();
    return id !== null && (this.threads.data()?.rows ?? []).some((r) => r.id === id) ? [id] : [];
  });

  constructor() {
    effect(() => {
      const id = this.id();
      untracked(() => {
        if (id === null) {
          for (const r of [this.overview, this.findings, this.threads, this.stack, this.modules]) r.reset();
          return;
        }
        void this.overview.load(() => this.backend.overview(id));
        void this.findings.load(() => this.backend.findings({ ids: [id] }));
      });
    });
    // Downloaded PDBs arrived: the engine read this dump again, so what is shown is read again (the tab stays).
    effect(() => {
      const changed = this.store.symbols();
      untracked(() => {
        const id = this.id();
        if (changed === null || id === null || changed.id !== id) return;
        void this.overview.load(() => this.backend.overview(id));
        void this.findings.load(() => this.backend.findings({ ids: [id] }));
        const thread = this.selectedThread();
        if (thread !== null) void this.stack.load(() => this.backend.stack({ id, threadId: thread }));
        if (this.tab() === 'modules') {
          const filter = this.moduleFilter().trim() || null;
          void this.modules.load(() => this.backend.modules({ id, offset: 0, limit: PAGE, filter }));
        }
      });
    });
    effect(() => {
      const id = this.id();
      const filter = this.threadFilter().trim() || null;
      if (id === null || this.tab() !== 'threads') return;
      untracked(() =>
        void this.threads.load(async () => {
          const page = await this.backend.threads({ id, offset: 0, limit: PAGE, filter });
          if (this.selectedThread() === null && page.rows[0]) this.selectThread(page.rows[0].id);
          return page;
        }),
      );
    });
    effect(() => {
      const id = this.id();
      const thread = this.selectedThread();
      untracked(() => (id !== null && thread !== null ? void this.stack.load(() => this.backend.stack({ id, threadId: thread })) : this.stack.reset()));
    });
    effect(() => {
      const id = this.id();
      const filter = this.moduleFilter().trim() || null;
      if (id === null || this.tab() !== 'modules') return;
      untracked(() => void this.modules.load(() => this.backend.modules({ id, offset: 0, limit: PAGE, filter })));
    });
  }

  protected pick(e: Event): void {
    this.store.setActiveDump(Number((e.target as HTMLSelectElement).value));
  }

  protected value(e: Event): string {
    return (e.target as HTMLInputElement).value;
  }

  selectThread(id: unknown): void {
    if (typeof id === 'number') {
      this.selectedThread.set(id);
      this.highlightFrame.set(null);
    }
  }

  /** Shows a thread's stack, with one frame marked. */
  showThread(id: number, frame: number | null): void {
    this.tab.set('threads');
    this.selectedThread.set(id);
    this.highlightFrame.set(frame);
  }

  protected showEvidence(f: DumpFinding): void {
    const t = f.evidence.threadId ?? f.evidence.threads[0] ?? null;
    if (t !== null) this.showThread(t, f.evidence.frame);
  }

  protected async explain(): Promise<void> {
    const id = this.id();
    if (id === null) return;
    try {
      await this.explainer.explain([id]);
    } catch (e) {
      this.findings.error.set(errorText(e));
    }
  }

  protected kindLabel(kind: string): string {
    return kind === 'full' ? $localize`:@@dumps.kind.full:full dump` : kind === 'withHeap' ? $localize`:@@dumps.kind.heap:minidump with heap` : $localize`:@@dumps.kind.mini:minidump`;
  }

  protected when(ms: number): string {
    return formatDateTime(ms, this.workspace.timezone(), false);
  }

  protected uptime(s: number): string {
    return formatDuration(s * 1000);
  }

  protected stateLabel(s: ThreadState): string {
    return STATE_LABEL[s];
  }

  protected trustLabel(t: FrameTrust): string {
    return TRUST_LABEL[t];
  }

  protected stackLabel(id: number): string {
    return $localize`:@@dumps.stack.label:Stack of thread ${id}:id:`;
  }

  protected readonly bytes = formatBytes;
  protected readonly headline = headlineText;
  protected readonly count = formatCount;
  protected readonly symbolLabel = symbolStatusLabel;
  protected readonly hint = symbolStatusHint;
  protected readonly problemLines = symbolProblemLines;
  protected readonly problemTotal = symbolProblemTotal;
}
