import { afterNextRender, ChangeDetectionStrategy, Component, computed, DestroyRef, ElementRef, inject, Injectable, Injector, signal, viewChild } from '@angular/core';
import { LifecycleBackend } from '../core/tools-backend';
import { AppStatus, Platform } from '../core/tools.types';
import { DIALOG_REF } from '../ui/dialog.service';
import { copyText } from '../tools/clipboard';
import { DialogService } from '../ui/dialog.service';
import { AppLogo } from '../shared/app-logo';
import { BUILD_INFO } from './build-info';
import { READ_ONLY_DETAIL, READ_ONLY_PROMISE } from '../shared/read-only';
import { LEGAL_SOURCE, LegalDoc } from './legal-source';
import { filterLines, LINE_HEIGHT, toLines, versionInfoText, VIEWPORT_HEIGHT, visibleRange } from './notices-text';

export { BUILD_INFO, type BuildInfo } from './build-info';

const DOC_TITLES: Record<LegalDoc, string> = {
  notices: $localize`:@@about.doc.notices:Third-party notices`,
  mit: 'LICENSE-MIT',
  apache: 'LICENSE-APACHE',
};

/** About Debrief: version, licence and the third-party notices in a searchable, virtualised viewer. */
@Component({
  selector: 'debrief-about',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [AppLogo],
  template: `
    <div class="id">
      <debrief-app-logo [size]="48" />
      <div>
        <h3 class="name">Debrief</h3>
        <p class="meta" i18n="@@about.meta">
          Version <strong class="version">{{ version() ?? unknown }}</strong>
          · commit <span class="mono commit">{{ build.commit }}</span>
          · built <span class="built">{{ build.date }}</span>
        </p>
      </div>
    </div>

    <p class="promise" data-testid="read-only-promise">
      <strong>{{ promise }}</strong> {{ promiseDetail }}
    </p>

    <p class="licence" i18n="@@about.licence">
      Licensed under <strong>MIT OR Apache-2.0</strong>, at your option.
      <button class="btn" type="button" [attr.aria-pressed]="doc() === 'mit'" (click)="show('mit')">View LICENSE-MIT</button>
      <button class="btn" type="button" [attr.aria-pressed]="doc() === 'apache'" (click)="show('apache')">View LICENSE-APACHE</button>
    </p>

    <div class="viewer-head">
      <h4>{{ title() }}</h4>
      @if (doc() !== 'notices') {
        <button class="btn ghost" type="button" (click)="show('notices')" i18n="@@about.thirdPartyNotices">Third-party notices</button>
      }
      <input class="input search" type="search" placeholder="Search this text" i18n-placeholder="@@about.search.placeholder" aria-label="Search notices" i18n-aria-label="@@about.search.label" [value]="query()" (input)="setQuery($any($event.target).value)" />
      <span class="count faint" role="status" i18n="@@about.lineCount">{{ shown().length }} of {{ lines().length }} lines</span>
    </div>
    @if (error(); as e) {
      <p class="err" role="alert">{{ e }} <button class="btn" type="button" (click)="show(doc())" i18n="@@about.retry">Retry</button></p>
    }
    <div #viewport class="viewport" tabindex="0" role="region" [attr.aria-label]="viewportLabel()" (scroll)="onScroll($event)">
      <div class="spacer" [style.height.px]="shown().length * lineHeight">
        <div class="rows" [style.transform]="'translateY(' + range().start * lineHeight + 'px)'">
          @for (row of rows(); track row.index) {
            <div class="line" [class.head]="row.text.startsWith('#')">{{ row.text }}</div>
          }
        </div>
      </div>
    </div>

    <div class="actions">
      <button class="btn" type="button" (click)="copy()" i18n="@@about.copyVersion">Copy version info…</button>
      <span class="faint" role="status">{{ copied() }}</span>
      <button class="btn primary close" type="button" (click)="ref?.close()" i18n="@@about.close">Close</button>
    </div>
  `,
  styles: `
    /* Fills the window: the notices take the remaining height and scroll inside; the footer stays pinned and the window itself never scrolls. */
    :host { display: flex; flex-direction: column; gap: 12px; min-width: 0; min-height: 0; height: 100%; }
    .id { display: flex; gap: 14px; align-items: center; }
    .name { margin: 0; font-size: var(--fs-lg); }
    .promise { margin: 0; padding: 8px 10px; border: 1px solid var(--border-2); border-inline-start: 3px solid var(--ok); border-radius: 6px; background: var(--bg-3); color: var(--text-2); font-size: var(--fs-sm); line-height: 1.45; }
    .meta, .licence { margin: 4px 0 0; color: var(--muted); display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
    .viewer-head { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
    h4 { margin: 0; font-size: var(--fs-sm); flex: 1 1 auto; }
    .search { flex: 0 1 220px; }
    .count { font-size: var(--fs-xs); }
    .viewport { flex: 1 1 320px; min-height: 120px; overflow: auto; border: 1px solid var(--border); border-radius: 6px; background: var(--bg-1, var(--bg-2)); }
    .spacer { position: relative; min-width: max-content; }
    .rows { position: absolute; inset-inline-start: 0; top: 0; }
    .line { height: 18px; line-height: 18px; padding: 0 8px; white-space: pre; font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text); }
    .line.head { font-weight: 600; color: var(--accent-text); }
    .err { color: var(--danger-text); margin: 0; }
    .actions { flex: none; display: flex; gap: 10px; align-items: center; padding-top: 8px; border-top: 1px solid var(--border); }
    .close { margin-inline-start: auto; }
  `,
})
export class AboutDialog {
  static readonly dialogId = 'about';
  private readonly legal = inject(LEGAL_SOURCE);
  private readonly lifecycle = inject(LifecycleBackend);
  private readonly injector = inject(Injector);
  private readonly destroyRef = inject(DestroyRef);
  protected readonly ref = inject(DIALOG_REF, { optional: true });
  private readonly viewportEl = viewChild<ElementRef<HTMLElement>>('viewport');
  /** Commit and date recorded at build time (`'unknown'` in dev builds and tests). */
  protected readonly build = inject(BUILD_INFO);
  protected readonly lineHeight = LINE_HEIGHT;
  protected readonly promise = READ_ONLY_PROMISE;
  protected readonly promiseDetail = READ_ONLY_DETAIL;

  protected readonly doc = signal<LegalDoc>('notices');
  protected readonly query = signal('');
  protected readonly lines = signal<readonly string[]>([]);
  protected readonly error = signal<string | null>(null);
  protected readonly copied = signal('');
  private readonly scrollTop = signal(0);
  /** The viewer's height in px, measured: the notices fill whatever the window leaves them. */
  private readonly viewportPx = signal(VIEWPORT_HEIGHT);
  private readonly status = signal<AppStatus | null>(null);
  private readonly platform = signal<Platform | null>(null);

  /** The version the engine reports for this build (never a constant of the UI). */
  protected readonly version = computed(() => this.status()?.version ?? null);
  protected readonly title = computed(() => DOC_TITLES[this.doc()]);
  protected readonly viewportLabel = computed(() => $localize`:@@about.viewport.label:${this.title()}:title: text`);
  protected readonly unknown = $localize`:@@about.unknown:unknown`;
  protected readonly shown = computed(() => filterLines(this.lines(), this.query()));
  protected readonly range = computed(() => visibleRange(this.scrollTop(), this.shown().length, this.viewportPx()));
  protected readonly rows = computed(() => {
    const { start, end } = this.range();
    return this.shown()
      .slice(start, end)
      .map((text, i) => ({ index: start + i, text }));
  });

  constructor() {
    // A dialog window has no session or lifecycle service of its own: it asks the engine directly.
    void this.lifecycle.appStatus().then((s) => this.status.set(s), () => undefined);
    void this.lifecycle.elevation().then((e) => this.platform.set(e.platform), () => undefined);
    void this.show('notices');
    afterNextRender(
      () => {
        const el = this.viewportEl()?.nativeElement;
        if (!el) return;
        const measure = () => this.viewportPx.set(el.clientHeight || VIEWPORT_HEIGHT);
        measure();
        if (typeof ResizeObserver === 'undefined') return;
        const observer = new ResizeObserver(measure);
        observer.observe(el);
        this.destroyRef.onDestroy(() => observer.disconnect());
      },
      { injector: this.injector },
    );
  }

  protected async show(doc: LegalDoc): Promise<void> {
    this.doc.set(doc);
    this.error.set(null);
    this.lines.set([]);
    this.scrollTop.set(0);
    try {
      const text = await this.legal.load(doc);
      if (this.doc() === doc) this.lines.set(toLines(text));
    } catch {
      if (this.doc() === doc) this.error.set($localize`:@@about.loadFailed:Could not load ${DOC_TITLES[doc]}:title:.`);
    }
  }

  protected setQuery(q: string): void {
    this.query.set(q);
    this.scrollTop.set(0);
  }

  protected onScroll(e: Event): void {
    this.scrollTop.set((e.target as HTMLElement).scrollTop);
  }

  protected async copy(): Promise<void> {
    const text = versionInfoText(this.version(), this.platform(), this.build);
    this.copied.set((await copyText(text))
        ? $localize`:@@about.copied:Copied`
        : $localize`:@@about.copyFailed:Copy failed — select the version text manually`);
  }
}

/** Opens the About dialog (Help → About Debrief, the "About Debrief" palette command). */
@Injectable({ providedIn: 'root' })
export class AboutService {
  private readonly dialogs = inject(DialogService);

  open(): void {
    this.dialogs.open(AboutDialog, { title: $localize`:@@about.title:About Debrief`, width: 720 });
  }
}
