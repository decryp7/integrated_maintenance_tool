import { ChangeDetectionStrategy, Component, computed, inject, Injectable, signal } from '@angular/core';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { copyText } from '../tools/clipboard';
import { DialogService } from '../ui/dialog.service';
import { BUILD_INFO } from './build-info';
import { READ_ONLY_DETAIL, READ_ONLY_PROMISE } from '../shared/read-only';
import { LEGAL_SOURCE, LegalDoc } from './legal-source';
import { filterLines, LINE_HEIGHT, toLines, versionInfoText, VIEWPORT_HEIGHT, visibleRange } from './notices-text';

export { BUILD_INFO, type BuildInfo } from './build-info';

const DOC_TITLES: Record<LegalDoc, string> = { notices: 'Third-party notices', mit: 'LICENSE-MIT', apache: 'LICENSE-APACHE' };

/** About IMT: version, licence and the third-party notices in a searchable, virtualised viewer. */
@Component({
  selector: 'imt-about',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <div class="id">
      <svg class="logo" viewBox="0 0 48 48" width="48" height="48" aria-hidden="true">
        <rect width="48" height="48" rx="10" fill="var(--accent)" />
        <path d="M10 14h8M14 14v20M10 34h8M22 34V14l6 12l6-12v20M38 14h4" fill="none" stroke="var(--accent-ink)" stroke-width="3" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
      <div>
        <h3 class="name">IMT — Integrated Maintenance Tool</h3>
        <p class="meta">
          Version <strong class="version">{{ version() ?? 'unknown' }}</strong>
          · commit <span class="mono commit">{{ build.commit }}</span>
          · built <span class="built">{{ build.date }}</span>
        </p>
      </div>
    </div>

    <p class="promise" data-testid="read-only-promise">
      <strong>{{ promise }}</strong> {{ promiseDetail }}
    </p>

    <p class="licence">
      Licensed under <strong>MIT OR Apache-2.0</strong>, at your option.
      <button class="btn" type="button" [attr.aria-pressed]="doc() === 'mit'" (click)="show('mit')">View LICENSE-MIT</button>
      <button class="btn" type="button" [attr.aria-pressed]="doc() === 'apache'" (click)="show('apache')">View LICENSE-APACHE</button>
    </p>

    <div class="viewer-head">
      <h4>{{ title() }}</h4>
      @if (doc() !== 'notices') {
        <button class="btn ghost" type="button" (click)="show('notices')">Third-party notices</button>
      }
      <input class="input search" type="search" placeholder="Search this text" aria-label="Search notices" [value]="query()" (input)="setQuery($any($event.target).value)" />
      <span class="count faint" role="status">{{ shown().length }} of {{ lines().length }} lines</span>
    </div>
    @if (error(); as e) {
      <p class="err" role="alert">{{ e }} <button class="btn" type="button" (click)="show(doc())">Retry</button></p>
    }
    <div class="viewport" tabindex="0" role="region" [attr.aria-label]="title() + ' text'" [style.height.px]="viewportHeight" (scroll)="onScroll($event)">
      <div class="spacer" [style.height.px]="shown().length * lineHeight">
        <div class="rows" [style.transform]="'translateY(' + range().start * lineHeight + 'px)'">
          @for (row of rows(); track row.index) {
            <div class="line" [class.head]="row.text.startsWith('#')">{{ row.text }}</div>
          }
        </div>
      </div>
    </div>

    <div class="actions">
      <button class="btn" type="button" (click)="copy()"><span>Copy version info</span></button>
      <span class="faint" role="status">{{ copied() }}</span>
    </div>
  `,
  styles: `
    :host { display: flex; flex-direction: column; gap: 12px; min-width: 0; }
    .id { display: flex; gap: 14px; align-items: center; }
    .name { margin: 0; font-size: 15px; }
    .promise { margin: 0; padding: 8px 10px; border: 1px solid var(--border-2); border-left: 3px solid var(--ok); border-radius: 6px; background: var(--bg-3); color: var(--text-2); font-size: 12px; line-height: 1.45; }
    .meta, .licence { margin: 4px 0 0; color: var(--muted); display: flex; flex-wrap: wrap; gap: 6px; align-items: center; }
    .viewer-head { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; }
    h4 { margin: 0; font-size: 12px; flex: 1 1 auto; }
    .search { flex: 0 1 220px; }
    .count { font-size: 11px; }
    .viewport { overflow: auto; border: 1px solid var(--border); border-radius: 6px; background: var(--bg-1, var(--bg-2)); }
    .spacer { position: relative; min-width: max-content; }
    .rows { position: absolute; left: 0; top: 0; }
    .line { height: 18px; line-height: 18px; padding: 0 8px; white-space: pre; font-family: var(--font-mono); font-size: 11.5px; color: var(--text); }
    .line.head { font-weight: 600; color: var(--accent); }
    .err { color: var(--danger-text); margin: 0; }
    .actions { display: flex; gap: 10px; align-items: center; }
  `,
})
export class AboutDialog {
  private readonly legal = inject(LEGAL_SOURCE);
  private readonly lifecycle = inject(LifecycleService);
  /** Commit and date recorded at build time (`'unknown'` in dev builds and tests). */
  protected readonly build = inject(BUILD_INFO);
  protected readonly lineHeight = LINE_HEIGHT;
  protected readonly viewportHeight = VIEWPORT_HEIGHT;
  protected readonly promise = READ_ONLY_PROMISE;
  protected readonly promiseDetail = READ_ONLY_DETAIL;

  protected readonly doc = signal<LegalDoc>('notices');
  protected readonly query = signal('');
  protected readonly lines = signal<readonly string[]>([]);
  protected readonly error = signal<string | null>(null);
  protected readonly copied = signal('');
  private readonly scrollTop = signal(0);

  protected readonly version = computed(() => this.lifecycle.status()?.version ?? null);
  protected readonly title = computed(() => DOC_TITLES[this.doc()]);
  protected readonly shown = computed(() => filterLines(this.lines(), this.query()));
  protected readonly range = computed(() => visibleRange(this.scrollTop(), this.shown().length));
  protected readonly rows = computed(() => {
    const { start, end } = this.range();
    return this.shown()
      .slice(start, end)
      .map((text, i) => ({ index: start + i, text }));
  });

  constructor() {
    void this.show('notices');
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
      if (this.doc() === doc) this.error.set(`Could not load ${DOC_TITLES[doc]}.`);
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
    const text = versionInfoText(this.version(), this.lifecycle.platform(), this.build);
    this.copied.set((await copyText(text)) ? 'Copied' : 'Copy failed — select the version text manually');
  }
}

/** Opens the About dialog (Help → About IMT, the "About IMT" palette command). */
@Injectable({ providedIn: 'root' })
export class AboutService {
  private readonly dialogs = inject(DialogService);

  open(): void {
    if (this.dialogs.stack().some((d) => d.component === AboutDialog)) return;
    this.dialogs.open(AboutDialog, { title: 'About IMT', width: 720 });
  }
}
