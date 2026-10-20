import { ChangeDetectionStrategy, Component, computed, inject, signal } from '@angular/core';
import { formatDuration } from '../core/format';
import { WorkspaceDataStore } from '../core/workspace/workspace-data.store';
import { plural } from '../i18n/locale';
import { Icon } from '../shared/icon';
import { Switch } from '../shared/switch';
import { AutoFocus } from '../shared/auto-focus';
import { LiveWindowControls } from '../live/live-window-controls';
import { ContextMenuService } from '../ui/context-menu.service';
import { DockService } from '../dock/dock.service';
import { LANE_MODES, LaneMode, TimelineScope } from './timeline-config';
import { TimelineInstanceStore } from './timeline-instance.store';
import { TIMELINE_SOURCE } from './timeline-source';
import { TimelineViews } from './timeline-views';
import { RangeZoom } from './zoom';
import { OverviewHeadline } from '../overview/overview-headline';

/**
 * The timeline's title bar (title and rename, hint, loading, live window, zoom history, menu) and, for an additional
 * timeline, its settings bar (events shown, lanes, sync).
 */
@Component({
  selector: 'imt-timeline-header',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, Switch, AutoFocus, LiveWindowControls, OverviewHeadline],
  host: { style: 'display: contents' },
  template: `
    <div class="head">
      @if (renaming()) {
        <input class="input title-input" imtAutoFocus aria-label="Timeline title" i18n-aria-label="@@timeline.panel.titleAria" maxlength="80" [value]="src.title()" (keydown)="renameKey($event)" (blur)="commitRename($event)" />
      } @else {
        <span class="section-title" [attr.data-testid]="src.main ? null : 'timeline-title'" (dblclick)="startRename()">{{ src.title() }}</span>
      }
      @if (inst && !inst.synced()) { <span class="unlinked" title="Zoom, pan and the time cursor are not linked to the main timeline" i18n-title="@@timeline.panel.unlinked.title" i18n="@@timeline.panel.unlinked">Unlinked</span> }
      @if (src.main) {
        <!-- The health check's headline, on the title row (the Overview stays one click away). -->
        <imt-overview-headline />
      }
      <span class="faint hint">{{ hint() }}</span>
      @if (src.loading()) { <span class="spinner" role="status" aria-label="Loading" i18n-aria-label="@@timeline.panel.loading"></span> }
      <span class="spacer"></span>
      @if (src.main) { <imt-live-window-controls /> }
      <div class="zoom" role="group" aria-label="Zoom" i18n-aria-label="@@timeline.panel.zoom">
        <button class="icon-btn" title="Previous range (Alt+←)" i18n-title="@@timeline.panel.back.title" aria-label="Previous range" i18n-aria-label="@@timeline.panel.back.aria" [disabled]="!zoom.canBack()" (click)="zoom.back()">←</button>
        <button class="icon-btn" title="Next range (Alt+→)" i18n-title="@@timeline.panel.forward.title" aria-label="Next range" i18n-aria-label="@@timeline.panel.forward.aria" [disabled]="!zoom.canForward()" (click)="zoom.forward()">→</button>
        <button class="icon-btn" title="Zoom in" i18n-title="@@timeline.panel.zoomIn.title" aria-label="Zoom in" i18n-aria-label="@@timeline.panel.zoomIn.aria" (click)="zoom.zoomIn()"><imt-icon name="plus" [size]="13" /></button>
        <button class="icon-btn" title="Zoom out" i18n-title="@@timeline.panel.zoomOut.title" aria-label="Zoom out" i18n-aria-label="@@timeline.panel.zoomOut.aria" (click)="zoom.zoomOut()"><imt-icon name="minus" [size]="13" /></button>
        <button class="btn" title="Fit all (double-click)" i18n-title="@@timeline.panel.fit.title" [disabled]="!zoom.isZoomed()" (click)="zoom.fit()" i18n="@@timeline.panel.fit"><imt-icon name="zoomOut" [size]="13" /> Fit</button>
      </div>
      <button class="icon-btn" type="button" title="Timeline menu" i18n-title="@@timeline.panel.menu.title" aria-label="Timeline menu" i18n-aria-label="@@timeline.panel.menu.aria" aria-haspopup="menu" (click)="openMenu($event)"><imt-icon name="chevronDown" [size]="13" /></button>
    </div>
    @if (inst; as s) {
      <div class="config" role="group" aria-label="Timeline settings" i18n-aria-label="@@timeline.config.aria">
        <label class="field">
          <span class="faint" i18n="@@timeline.config.events">Events</span>
          <select class="select" aria-label="Events shown" i18n-aria-label="@@timeline.config.eventsShown" [value]="s.config().scope" (change)="s.setScope(asScope($event))">
            <option value="global" i18n="@@timeline.config.scope.global">Workspace filter</option>
            <option value="own" i18n="@@timeline.config.scope.own">Own query</option>
          </select>
        </label>
        @if (s.config().scope === 'own') {
          <input
            class="input mono ql"
            aria-label="Timeline query"
            i18n-aria-label="@@timeline.config.queryAria"
            placeholder="e.g. level >= error and MachineName = WEB01"
            i18n-placeholder="@@timeline.config.queryPlaceholder"
            maxlength="4000"
            [value]="s.config().ql"
            (keydown.enter)="s.setQl(valueOf($event))"
            (change)="s.setQl(valueOf($event))"
          />
        }
        <label class="field">
          <span class="faint" i18n="@@timeline.config.lanes">Lanes</span>
          <select class="select" aria-label="Lanes" i18n-aria-label="@@timeline.config.lanesAria" (change)="s.setLanes(asLanes($event))">
            @for (m of laneModes; track m.mode) { <option [value]="m.mode" [selected]="m.mode === s.config().lanes">{{ m.label }}</option> }
          </select>
        </label>
        @if (s.config().lanes === 'field') {
          <select class="select" aria-label="Lane field" i18n-aria-label="@@timeline.config.laneField" (change)="s.setLanes('field', valueOf($event))">
            <option value="" [selected]="!s.config().laneField" i18n="@@timeline.config.chooseField">Choose a field…</option>
            @for (f of data.fieldNames(); track f) { <option [value]="f" [selected]="f === s.config().laneField">{{ f }}</option> }
          </select>
        }
        <span class="spacer"></span>
        <imt-switch size="small" [checked]="s.synced()" (checkedChange)="s.setSync($event)" i18n="@@timeline.config.syncTime">Sync time</imt-switch>
      </div>
    }
  `,
  styles: `
    .head { height: 32px; flex: none; display: flex; align-items: center; gap: 12px; padding: 0 14px; }
    .hint { min-width: 0; font-size: var(--fs-sm); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
    .section-title { white-space: nowrap; }
    .title-input { width: 200px; height: 24px; padding: 0 6px; }
    .unlinked { flex: none; font-size: var(--fs-xs); padding: 0 6px; border-radius: 8px; border: 1px dashed var(--border-3); color: var(--text-2); }
    .config { flex: none; display: flex; flex-wrap: wrap; align-items: center; gap: 6px 12px; padding: 0 14px 6px; font-size: var(--fs-sm); }
    .config .field { display: inline-flex; align-items: center; gap: 6px; }
    .config .select { height: 24px; }
    .config .ql { flex: 1 1 260px; min-width: 160px; height: 24px; padding: 0 6px; }
    .spacer { flex: 1; }
    .spinner { width: 10px; height: 10px; border-radius: 50%; border: 2px solid var(--border-3); border-top-color: var(--accent); animation: spin 0.8s linear infinite; flex: none; }
    @keyframes spin { to { transform: rotate(360deg); } }
    .zoom { display: flex; align-items: center; gap: 2px; }
    .zoom .icon-btn:disabled { opacity: 0.35; cursor: default; }
  `,
})
export class TimelineHeader {
  protected readonly src = inject(TIMELINE_SOURCE);
  /** The additional timeline's store (settings bar, rename, sync); null for the main timeline. */
  protected readonly inst = this.src.main ? null : inject(TimelineInstanceStore);
  protected readonly data = inject(WorkspaceDataStore);
  private readonly views = inject(TimelineViews);
  private readonly dock = inject(DockService);
  private readonly menu = inject(ContextMenuService);
  protected readonly laneModes = LANE_MODES;
  protected readonly renaming = signal(false);

  /** The zoom controller in effect (the workspace's while linked). */
  protected get zoom(): RangeZoom {
    return this.src.zoom();
  }

  /** "3 instances · bucket 1 min · drag to zoom …" once a timeline arrived. */
  protected readonly hint = computed(() => {
    const t = this.src.timeline();
    if (!t) return '';
    const n = t.lanes.length;
    const bucket = formatDuration(t.bucketMs ?? 0);
    return this.src.main
      ? plural(n, {
          one: $localize`:@@timeline.hint.instances.one:${n}:count: instance · bucket ${bucket}:bucket: · drag to zoom · ⌘/Ctrl+wheel zoom · Shift+wheel or Space+drag pan`,
          other: $localize`:@@timeline.hint.instances.other:${n}:count: instances · bucket ${bucket}:bucket: · drag to zoom · ⌘/Ctrl+wheel zoom · Shift+wheel or Space+drag pan`,
        })
      : plural(n, {
          one: $localize`:@@timeline.hint.lanes.one:${n}:count: lane · bucket ${bucket}:bucket: · drag to zoom · ⌘/Ctrl+wheel zoom · Shift+wheel or Space+drag pan`,
          other: $localize`:@@timeline.hint.lanes.other:${n}:count: lanes · bucket ${bucket}:bucket: · drag to zoom · ⌘/Ctrl+wheel zoom · Shift+wheel or Space+drag pan`,
        });
  });

  protected valueOf(e: Event): string {
    return (e.target as HTMLInputElement | HTMLSelectElement).value;
  }

  protected asScope(e: Event): TimelineScope {
    return this.valueOf(e) === 'own' ? 'own' : 'global';
  }

  protected asLanes(e: Event): LaneMode {
    const v = this.valueOf(e);
    return LANE_MODES.find((m) => m.mode === v)?.mode ?? 'instance';
  }

  protected startRename(): void {
    if (this.inst) this.renaming.set(true);
  }

  protected renameKey(e: KeyboardEvent): void {
    if (e.key === 'Enter') this.commitRename(e);
    else if (e.key === 'Escape') {
      e.preventDefault();
      this.renaming.set(false);
    }
  }

  protected commitRename(e: Event): void {
    if (!this.renaming()) return;
    this.renaming.set(false);
    this.inst?.rename(this.valueOf(e));
  }

  /** The panel's own menu: open another timeline; for an additional one also rename, link or unlink, close. */
  protected openMenu(e: MouseEvent): void {
    const anchor = (e.currentTarget as HTMLElement).getBoundingClientRect();
    const inst = this.inst;
    this.menu.open(anchor.left, anchor.bottom, [
      { label: $localize`:@@timeline.menu.new:New timeline`, run: () => void this.views.open() },
      ...(inst
        ? [
            { label: $localize`:@@timeline.menu.rename:Rename…`, run: () => this.renaming.set(true) },
            { label: inst.synced() ? $localize`:@@timeline.menu.unlink:Unlink time from the main timeline` : $localize`:@@timeline.menu.sync:Sync time with the main timeline`, run: () => inst.setSync(!inst.synced()) },
            { label: $localize`:menu item, closes the timeline panel@@timeline.menu.close:Close`, run: () => this.dock.close(inst.id) },
          ]
        : []),
    ]);
  }
}
