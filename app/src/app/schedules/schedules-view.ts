import { ChangeDetectionStrategy, Component, computed, inject, OnInit } from '@angular/core';
import { ActionKind, AgentFieldSchema, ScheduleFile, TriggerKind } from '../core/agent.types';
import { Icon } from '../shared/icon';
import { TimingKind, timingOf } from './schedule-model';
import { SchedulesStore } from './schedules.store';
import { SchemaField } from './schema-field';
import { plural } from '../i18n/locale';

/**
 * "Collection schedules": a schema-driven editor of the headless agent's `imt-agent.toml`.
 * Validation is the agent's; its issues are shown next to the fields they concern.
 */
@Component({
  selector: 'imt-schedules-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [Icon, SchemaField],
  template: `
    <div class="scroll" role="region" aria-label="Collection schedules" i18n-aria-label="@@schedules.region" tabindex="0">
      <p class="intro" i18n="@@schedules.intro">
        Schedules for <span class="mono">imt-agent</span>, the headless collector that runs as a service on servers. Each run writes a folder with a
        manifest; see the Runs view. Install it with <span class="mono">imt-agent service install</span> (Windows),
        <span class="mono">print-unit</span> (systemd) or <span class="mono">print-plist</span> (launchd).
      </p>

      <div class="inline">
        <label class="lbl grow"><ng-container i18n="@@schedules.configFile">Config file</ng-container>
          <input class="input mono" [value]="store.path()" placeholder="Default: IMT's config folder" i18n-placeholder="@@schedules.configFile.placeholder" (input)="store.setPath(val($event))" />
        </label>
        <button type="button" class="btn" [disabled]="!!store.busy()" (click)="store.load()" i18n="@@schedules.load"><imt-icon name="refresh" [size]="12" />Load</button>
      </div>

      @if (store.error(); as e) {
        <div class="err" role="alert">{{ e }} <button type="button" class="icon-btn" aria-label="Dismiss" i18n-aria-label="@@schedules.dismiss" (click)="store.dismissError()"><imt-icon name="close" [size]="12" /></button></div>
      }
      @if (store.notice(); as n) { <div class="note" role="status">{{ n }}</div> }
      @if (topIssues().length) {
        <div class="err" role="alert" data-testid="file-issues">@for (m of topIssues(); track m) { <div>{{ m }}</div> }</div>
      }

      @if (store.config(); as cfg) {
        @if (store.schema(); as schema) {
          <section aria-labelledby="sch-agent">
            <h2 id="sch-agent" class="section-title" i18n="@@schedules.agent">Agent</h2>
            <div class="grid">
              @for (f of schema.agent; track f.key) {
                <imt-schema-field [field]="f" [value]="$any(cfg.agent)[f.key]" [issues]="issues('agent.' + f.key)" (valueChange)="store.setAgent(f.key, $event, f.required)" />
              }
            </div>
            <div class="grid">
              @for (f of schema.log; track f.key) {
                <imt-schema-field [field]="f" [value]="$any(cfg.agent.log)[f.key]" [issues]="issues('agent.log.' + f.key)" (valueChange)="store.setLog(f.key, $event)" />
              }
            </div>
          </section>

          <section aria-labelledby="sch-retention">
            <h2 id="sch-retention" class="section-title" i18n="@@schedules.retention">Retention</h2>
            <div class="grid">
              @for (f of schema.retention; track f.key) {
                <imt-schema-field [field]="f" [value]="$any(cfg.retention)[f.key]" [issues]="issues('retention.' + f.key)" (valueChange)="store.setRetention(f.key, $event)" />
              }
            </div>
          </section>

          <section aria-labelledby="sch-list">
            <h2 id="sch-list" class="section-title" i18n="@@schedules.list">Schedules ({{ cfg.schedules.length }})</h2>
            @for (s of cfg.schedules; track $index; let i = $index) {
              <fieldset class="card" [attr.data-testid]="'schedule-' + i">
                <legend>{{ s.name || unnamed }}</legend>
                @for (m of issues('schedules[' + i + ']') ?? []; track m) { <div class="problem" role="alert">{{ m }}</div> }
                <div class="grid">
                  @for (f of schema.schedule; track f.key) {
                    <imt-schema-field [field]="f" [value]="$any(s)[f.key]" [issues]="issues('schedules[' + i + '].' + f.key)" (valueChange)="store.setScheduleKey(i, f.key, $event, f.required)" />
                  }
                  <label class="lbl col"><ng-container i18n="how a schedule is started|@@schedules.runs">Runs</ng-container>
                    <select class="select" [value]="timing(s)" (change)="store.setTiming(i, $any(val($event)))">
                      @for (t of schema.timing; track t.kind) { <option [value]="t.kind" [selected]="t.kind === timing(s)">{{ t.label }}</option> }
                    </select>
                  </label>
                </div>
                @if (timing(s) === 'trigger' && s.trigger) {
                  <div class="grid">
                    <label class="lbl col"><ng-container i18n="@@schedules.trigger">Trigger</ng-container>
                      <select class="select" [value]="s.trigger.kind" (change)="store.setTriggerKind(i, $any(val($event)))">
                        @for (t of schema.triggers; track t.kind) { <option [value]="t.kind" [selected]="t.kind === s.trigger.kind">{{ t.label }}</option> }
                      </select>
                    </label>
                    @for (f of triggerFields(s); track f.key) {
                      <imt-schema-field [field]="f" [value]="$any(s.trigger)[f.key]" [issues]="issues('schedules[' + i + '].trigger.' + f.key)" (valueChange)="store.setTriggerKey(i, f.key, $event, f.required)" />
                    }
                  </div>
                } @else {
                  <div class="grid">
                    @for (f of timingFields(s); track f.key) {
                      <imt-schema-field [field]="f" [value]="$any(s)[f.key]" [issues]="issues('schedules[' + i + '].' + f.key)" (valueChange)="store.setScheduleKey(i, f.key, $event, f.required)" />
                    }
                  </div>
                }

                <h3 class="sub" i18n="@@schedules.actions">Actions</h3>
                @for (m of issues('schedules[' + i + '].actions') ?? []; track m) { <div class="problem" role="alert">{{ m }}</div> }
                @for (a of s.actions; track $index; let j = $index) {
                  <div class="action" [attr.data-testid]="'action-' + i + '-' + j">
                    <div class="inline">
                      <label class="lbl col grow"><ng-container i18n="@@schedules.actionN">Action {{ j + 1 }}</ng-container>
                        <select class="select" [attr.aria-label]="actionKindLabel(j)" [value]="a.kind" (change)="store.setActionKind(i, j, $any(val($event)))">
                          @for (v of schema.actions; track v.kind) { <option [value]="v.kind" [selected]="v.kind === a.kind">{{ v.label }}</option> }
                        </select>
                      </label>
                      <button type="button" class="icon-btn" [attr.aria-label]="removeActionLabel(j, s.name)" title="Remove action" i18n-title="@@schedules.removeAction.title" (click)="store.removeAction(i, j)"><imt-icon name="trash" [size]="13" /></button>
                    </div>
                    @for (m of issues('schedules[' + i + '].actions[' + j + ']') ?? []; track m) { <div class="problem" role="alert">{{ m }}</div> }
                    <div class="grid">
                      @for (f of actionFields(a.kind); track f.key) {
                        <imt-schema-field [field]="f" [value]="$any(a)[f.key]" [issues]="issues('schedules[' + i + '].actions[' + j + '].' + f.key)" (valueChange)="store.setActionKey(i, j, f.key, $event, f.required)" />
                      }
                    </div>
                  </div>
                }
                <div class="inline">
                  <button type="button" class="btn" (click)="store.addAction(i)" i18n="@@schedules.addAction"><imt-icon name="plus" [size]="12" />Add action</button>
                  <span class="spacer"></span>
                  <button type="button" class="btn danger" (click)="store.removeSchedule(i)" i18n="@@schedules.removeSchedule"><imt-icon name="trash" [size]="12" />Remove schedule</button>
                </div>
              </fieldset>
            }
            <div><button type="button" class="btn" (click)="store.addSchedule()" i18n="@@schedules.addSchedule"><imt-icon name="plus" [size]="12" />Add schedule</button></div>
          </section>

          <div class="inline footer">
            <button type="button" class="btn" [disabled]="!!store.busy()" (click)="store.validate()" i18n="@@schedules.validate">Validate</button>
            <button type="button" class="btn primary" [disabled]="!!store.busy()" (click)="store.save()" i18n="@@schedules.save">Save</button>
            <span class="faint" role="status" data-testid="schedules-state">{{ stateText() }}</span>
          </div>
        }
      } @else if (!store.busy() && !store.error()) {
        <p class="faint" i18n="@@schedules.loadHint">Load the configuration to edit it.</p>
      }
    </div>
  `,
  styles: `
    :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .scroll { flex: 1; min-height: 0; overflow: auto; padding: 14px 18px; display: flex; flex-direction: column; gap: 12px; max-width: 980px; }
    .scroll:focus-visible { outline-offset: -2px; }
    section { display: flex; flex-direction: column; gap: 8px; }
    h2 { margin: 0; }
    .sub { margin: 4px 0 0; font-size: var(--fs-sm); color: var(--text-2); }
    .intro { margin: 0; color: var(--text-2); line-height: 1.5; }
    .grid { display: flex; flex-wrap: wrap; gap: 10px 12px; align-items: flex-start; }
    .card { border: 1px solid var(--border-3); border-radius: 8px; background: var(--bg-2); padding: 8px 12px 12px; display: flex; flex-direction: column; gap: 8px; margin: 0; }
    legend { padding: 0 4px; font-weight: 600; color: var(--text); }
    .action { border-left: 2px solid var(--border-3); padding-left: 10px; display: flex; flex-direction: column; gap: 6px; }
    .inline { display: flex; gap: 8px; align-items: flex-end; flex-wrap: wrap; }
    .grow { flex: 1; }
    .spacer { flex: 1; }
    .lbl { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-xs); color: var(--muted); }
    .col { min-width: 160px; }
    .problem { color: var(--danger-text); font-size: var(--fs-sm); }
    .err { padding: 6px 10px; border-radius: 6px; border: 1px solid var(--danger-text); background: var(--bg-3); color: var(--danger-text); font-size: var(--fs-sm); display: flex; flex-direction: column; gap: 2px; }
    .note { padding: 6px 10px; border-radius: 6px; border: 1px solid var(--ok); background: var(--bg-3); font-size: var(--fs-sm); }
    .scroll { scroll-padding-bottom: 56px; }
    .footer { position: sticky; bottom: 0; z-index: 1; background: var(--bg-1); padding: 8px 0; margin-top: 4px; border-top: 1px solid var(--border); align-items: center; }
  `,
})
export class SchedulesView implements OnInit {
  protected readonly store = inject(SchedulesStore);
  protected readonly unnamed = $localize`:@@schedules.unnamed:Unnamed schedule`;

  protected actionKindLabel = (j: number) => $localize`:@@schedules.actionKindN:Kind of action ${j + 1}:n:`;
  protected removeActionLabel(j: number, name: string): string {
    const schedule = name || $localize`:@@schedules.scheduleFallback:schedule`;
    return $localize`:@@schedules.removeActionN:Remove action ${j + 1}:n: of ${schedule}:schedule:`;
  }

  /** Issues for the whole file (empty path) or keys the form does not show. */
  protected readonly topIssues = computed(() => this.store.issues().filter((i) => !i.path || i.path === 'version' || i.path === 'schedules').map((i) => (i.path ? `${i.path}: ${i.message}` : i.message)));

  protected readonly stateText = computed(() => {
    const n = this.store.issues().length;
    if (this.store.busy()) return this.store.busy();
    if (n) {
      return plural(n, {
        one: $localize`:@@schedules.state.problems.one:${n}:count: problem`,
        other: $localize`:@@schedules.state.problems.other:${n}:count: problems`,
      });
    }
    if (this.store.dirty()) return $localize`:@@schedules.state.unsaved:Unsaved changes`;
    return this.store.exists() ? $localize`:@@schedules.state.saved:Saved` : $localize`:@@schedules.state.notSaved:Not saved yet`;
  });

  ngOnInit(): void {
    if (!this.store.config()) void this.store.load();
  }

  protected issues(path: string): string[] | undefined {
    return this.store.issuesByPath().get(path);
  }

  protected timing(s: ScheduleFile): TimingKind {
    return timingOf(s);
  }

  protected timingFields(s: ScheduleFile): AgentFieldSchema[] {
    return this.store.schema()?.timing.find((t) => t.kind === timingOf(s))?.fields ?? [];
  }

  protected triggerFields(s: ScheduleFile): AgentFieldSchema[] {
    const kind: TriggerKind | undefined = s.trigger?.kind;
    return this.store.schema()?.triggers.find((t) => t.kind === kind)?.fields ?? [];
  }

  protected actionFields(kind: ActionKind): AgentFieldSchema[] {
    return this.store.schema()?.actions.find((a) => a.kind === kind)?.fields ?? [];
  }

  protected val = (e: Event) => (e.target as HTMLInputElement).value;
}
