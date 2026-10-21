import { ChangeDetectionStrategy, Component, computed, inject, Injectable, linkedSignal, signal } from '@angular/core';
import { WorkspaceStore } from '../core/workspace.store';
import { Icon } from '../shared/icon';
import { RECENT_STORE } from './recent-store';
import { DemoService } from '../onboarding/demo.service';
import { HelpHint } from '../ui/help-hint';
import { READ_ONLY_DETAIL, READ_ONLY_PROMISE } from '../shared/read-only';

/**
 * Whether the welcome screen is shown: whenever the workspace is empty, or when asked for explicitly. It covers the
 * dock, so it steps aside when the user opens a view (Jobs, Notifications, AI set-up need no files) until the
 * workspace changes or the welcome screen is asked for again.
 */
@Injectable({ providedIn: 'root' })
export class WelcomeService {
  private readonly store = inject(WorkspaceStore);
  private readonly forced = signal(false);
  /** A view was opened over the empty workspace; reset whenever the number of sources changes. */
  private readonly aside = linkedSignal({ source: () => this.store.summary()?.sources.length ?? 0, computation: () => false });

  readonly visible = computed(() => {
    const summary = this.store.summary();
    return this.forced() || (summary !== null && summary.sources.length === 0 && !this.aside());
  });

  show(): void {
    this.aside.set(false);
    this.forced.set(true);
  }
  hide(): void {
    this.forced.set(false);
  }
  /** The user opened a view: show the dock. */
  stepAside(): void {
    this.forced.set(false);
    this.aside.set(true);
  }
}

/** Last two path segments, e.g. "web01/orders.clef". */
export function shortPath(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).slice(-2).join('/') || path;
}

@Component({
  selector: 'imt-welcome',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, HelpHint],
  template: `
    <section class="welcome" aria-labelledby="welcome-title">
      <h1 id="welcome-title" i18n="@@welcome.title">Welcome to IMT</h1>
      <p class="lead" i18n="@@welcome.lead">Open log files or a whole folder and explore what happened, instance by instance.</p>
      <div class="actions">
        <button class="btn primary big" type="button" (click)="store.pickAndOpen(true)" i18n="@@welcome.openFolder"><imt-icon name="folder" [size]="15" /> Open folder…</button>
        <button class="btn big" type="button" (click)="store.pickAndOpen(false)" i18n="@@welcome.openFiles"><imt-icon name="file" [size]="15" /> Open files…</button>
        <button class="btn big" type="button" data-testid="try-demo" [disabled]="demo.loading()" (click)="demo.start()"><imt-icon name="timeline" [size]="15" /> {{ demoLabel() }}</button>
      </div>
      <p class="safe" data-testid="read-only-promise"><imt-icon name="check" [size]="14" /> {{ promise }} <imt-help label="Your files are safe" i18n-label="@@welcome.safe.helpLabel" [text]="detail" /></p>
      <p class="faint hint" i18n="@@welcome.hint">Or drag and drop files or folders anywhere in the window. The format is detected from the content, not the extension.</p>

      @if (recent().length) {
        <div class="recent">
          <h2 class="section-title" i18n="@@welcome.recent">Recent</h2>
          <ul>
            @for (p of recent(); track p) {
              <li><button class="link mono" type="button" [title]="p" (click)="open(p)">{{ short(p) }}</button></li>
            }
          </ul>
        </div>
      }

      <h2 class="section-title how" i18n="@@welcome.how.title">How IMT works</h2>
      <ol class="steps" aria-label="How IMT works" i18n-aria-label="@@welcome.how.label">
        <li><strong i18n="@@welcome.how.open.title">1 · Open</strong><span i18n="@@welcome.how.open.text">Add log files or a folder.</span></li>
        <li><strong i18n="@@welcome.how.filter.title">2 · Filter</strong><span i18n="@@welcome.how.filter.text">Search, pick levels, add field filters.</span></li>
        <li><strong i18n="@@welcome.how.timeline.title">3 · Timeline</strong><span i18n="@@welcome.how.timeline.text">See instances over time; drag to zoom.</span></li>
        <li><strong i18n="@@welcome.how.inspect.title">4 · Inspect</strong><span i18n="@@welcome.how.inspect.text">Click an event to see every property.</span></li>
      </ol>
      @if (workspaceHasSources()) {
        <button class="btn" type="button" (click)="welcome.hide()" i18n="@@welcome.back">Back to workspace</button>
      }
    </section>
  `,
  styles: `
    :host { position: absolute; inset: 0; z-index: 5; overflow: auto; background: var(--bg-1); display: flex; justify-content: center; }
    .welcome { width: min(720px, 100%); padding: 48px 24px; display: flex; flex-direction: column; gap: 14px; align-items: flex-start; }
    h1 { margin: 0; font-size: var(--fs-xl); font-weight: 600; }
    .lead { margin: 0; color: var(--text-2); font-size: var(--fs-lg); }
    .actions { display: flex; gap: 10px; margin-top: 6px; }
    .big { height: 34px; padding: 0 16px; gap: 8px; }
    .hint { margin: 0; font-size: var(--fs-sm); }
    .safe { margin: 0; display: flex; align-items: center; gap: 6px; font-size: var(--fs-md); color: var(--text-2); }
    .safe imt-icon { color: var(--ok); }
    .recent ul { margin: 6px 0 0; padding: 0; list-style: none; display: flex; flex-direction: column; gap: 2px; }
    .link { border: 0; background: transparent; color: var(--chip-text); cursor: pointer; padding: 3px 0; text-align: left; font-size: var(--fs-sm); }
    .link:hover { text-decoration: underline; }
    .how { margin-top: 18px; }
    .steps { margin: 0; padding: 0; list-style: none; display: grid; grid-template-columns: repeat(4, 1fr); gap: 10px; width: 100%; }
    .steps li { display: flex; flex-direction: column; gap: 4px; padding: 10px 12px; border: 1px solid var(--border-2); border-radius: 8px; background: var(--bg-2); color: var(--muted); font-size: var(--fs-sm); }
    .steps strong { color: var(--text); font-size: var(--fs-md); }
  `,
})
export class Welcome {
  protected readonly store = inject(WorkspaceStore);
  protected readonly welcome = inject(WelcomeService);
  private readonly recentStore = inject(RECENT_STORE);
  protected readonly demo = inject(DemoService);
  protected readonly promise = READ_ONLY_PROMISE;
  protected readonly detail = READ_ONLY_DETAIL;
  protected readonly short = shortPath;
  protected readonly demoLabel = computed(() => (this.demo.loading() ? $localize`:@@welcome.demo.opening:Opening the demo…` : $localize`:@@welcome.demo:Try a demo`));

  protected readonly workspaceHasSources = computed(() => (this.store.summary()?.sources.length ?? 0) > 0);
  protected readonly recent = computed(() => {
    // Opening files adds to the list; appended live events do not.
    this.store.sourcesVersion();
    return this.recentStore.list();
  });

  protected open(path: string): void {
    this.welcome.hide();
    void this.store.openPaths([path]);
  }
}
