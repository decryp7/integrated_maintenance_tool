import { ChangeDetectionStrategy, Component, computed, inject, OnInit, signal } from '@angular/core';
import { SEVERITIES } from '../core/api.types';
import { DIALOG_FORM_STYLES } from '../context/dialog-styles';
import { DIALOG_REF } from '../ui/dialog.service';
import { severityLabel } from './alerts.store';
import { Detector, DETECTOR_LABELS, DETECTORS, errorTarget, ErrorTarget, LiveRuleDraft, LiveSettingsDraft, newRule } from './live-rules.model';
import { LiveRulesProfile, RulesFailure } from './live-rules.profile';
import { QlInput } from './ql-input';

type Phase = 'loading' | 'ready' | 'busy';

/**
 * Edits `[live]`: which detectors raise alerts, their timing, and the `[[live.rule]]` alert rules (a
 * query-language condition matched N times within a window). Validated by the engine before it is saved;
 * the engine's message is shown next to the rule or field it is about.
 */
@Component({
  selector: 'debrief-alert-rules-editor',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [QlInput],
  template: `
    @if (loadError(); as e) {
      <p class="err" role="alert" i18n="@@live.rulesEditor.loadFailed">The profile could not be loaded: {{ e }}</p>
    } @else if (draft(); as d) {
      <fieldset class="group" [class.invalid]="isField('alerts')">
        <legend i18n="@@live.rulesEditor.detectors">Detectors that raise alerts</legend>
        <div class="checks">
          @for (k of detectors; track k) {
            <label class="check"><input type="checkbox" [checked]="d.detectors.includes(k)" (change)="toggleDetector(k, $any($event.target).checked)" />{{ detectorLabels[k] }}</label>
          }
        </div>
        @if (isField('alerts')) { <div class="err">{{ failure()!.message }}</div> }
      </fieldset>

      <div class="row">
        <label class="lbl" [class.invalid]="isField('silence_after')"><ng-container i18n="@@live.rulesEditor.silenceAfter">Silence after</ng-container>
          <input class="input mono" [value]="d.silenceAfter" (change)="patch({ silenceAfter: $any($event.target).value })" placeholder="60s" />
        </label>
        <label class="lbl" [class.invalid]="isField('cooldown')"><ng-container i18n="@@live.rulesEditor.cooldown">Cooldown</ng-container>
          <input class="input mono" [value]="d.cooldown" (change)="patch({ cooldown: $any($event.target).value })" placeholder="5m" />
        </label>
        <label class="lbl" [class.invalid]="isField('window')"><ng-container i18n="@@live.rulesEditor.detectorWindow">Detector window</ng-container>
          <input class="input mono" [value]="d.window" (change)="patch({ window: $any($event.target).value })" placeholder="5m" />
        </label>
        <label class="lbl" [class.invalid]="isField('clock')"><ng-container i18n="@@live.rulesEditor.nowIs">"Now" is</ng-container>
          <select class="select" (change)="patch({ clock: $any($event.target).value })">
            <option value="data" [selected]="d.clock === 'data'" i18n="@@live.rulesEditor.clockData">the newest event</option>
            <option value="wall" [selected]="d.clock === 'wall'" i18n="@@live.rulesEditor.clockWall">the system clock</option>
          </select>
        </label>
      </div>
      @if (fieldError(); as e) { <div class="err">{{ e }}</div> }

      <div class="rules-head">
        <span class="section-title" i18n="@@live.rulesEditor.rulesTitle">Alert rules</span>
        <span class="faint small" i18n="@@live.rulesEditor.rulesHelp">Alert when the condition matches at least <em>threshold</em> events within the window.</span>
        <span class="spacer"></span>
        <button type="button" class="btn" (click)="addRule()" i18n="@@live.rulesEditor.addRule">+ Add rule</button>
      </div>
      <div class="rules">
        @for (r of d.rules; track $index; let i = $index) {
          <fieldset class="rule" [class.invalid]="isRule(i)" [attr.aria-invalid]="isRule(i) ? 'true' : null">
            <legend class="sr" i18n="@@live.rulesEditor.ruleLegend">Rule {{ i + 1 }}</legend>
            <div class="row">
              <label class="lbl name"><ng-container i18n="@@live.rulesEditor.ruleName">Name</ng-container><input class="input" [value]="r.name" (change)="patchRule(i, { name: $any($event.target).value })" /></label>
              <label class="lbl num"><ng-container i18n="@@live.rulesEditor.threshold">Threshold</ng-container><input class="input mono" type="number" min="1" [value]="r.threshold" (change)="patchRule(i, { threshold: +$any($event.target).value })" /></label>
              <label class="lbl num"><ng-container i18n="@@live.rulesEditor.ruleWindow">Window</ng-container><input class="input mono" [value]="r.window" (change)="patchRule(i, { window: $any($event.target).value })" placeholder="1m" /></label>
              <label class="lbl num"><ng-container i18n="@@live.rulesEditor.severity">Severity</ng-container>
                <select class="select" (change)="patchRule(i, { severity: $any($event.target).value })">
                  @for (s of severities; track s) { <option [value]="s" [selected]="s === r.severity">{{ sevLabel(s) }}</option> }
                </select>
              </label>
              <button type="button" class="icon-btn rm" [attr.aria-label]="removeLabel(r.name)" [title]="removeLabel(r.name)" (click)="removeRule(i)">×</button>
            </div>
            <div class="lbl"><ng-container i18n="@@live.rulesEditor.when">When</ng-container>
              <debrief-ql-input [value]="r.when" [label]="conditionLabel(i)" (valueChange)="patchRule(i, { when: $event })" />
            </div>
            @if (isRule(i)) { <div class="err" role="alert">{{ failure()!.message }}</div> }
          </fieldset>
        } @empty {
          <p class="faint small" i18n="@@live.rulesEditor.noRules">No alert rules. The detectors above still raise alerts.</p>
        }
      </div>

      @if (generalError(); as f) {
        <div class="err" role="alert">
          {{ f.message }}
          @if (f.line) { <div class="box" i18n="@@live.rulesEditor.lineExcerpt">line {{ f.line }}: {{ f.excerpt }}</div> }
        </div>
      }
      @if (validated()) { <p class="ok" role="status" i18n="@@live.rulesEditor.valid">The rules are valid.</p> }

      <div class="foot">
        <button type="button" class="btn ghost" (click)="ref.close()" i18n="@@live.rulesEditor.cancel">Cancel</button>
        <button type="button" class="btn" [disabled]="phase() !== 'ready'" (click)="validate()" i18n="@@live.rulesEditor.validate">Validate</button>
        <button type="button" class="btn primary" [disabled]="phase() !== 'ready'" (click)="save()" i18n="@@live.rulesEditor.save">Save</button>
      </div>
    } @else {
      <p class="faint" role="status" i18n="@@live.rulesEditor.loading">Loading the profile…</p>
    }
  `,
  styles: [
    DIALOG_FORM_STYLES,
    `
      .group { border: 1px solid var(--border); border-radius: 6px; padding: 8px 10px; margin: 0; }
      .group legend { font-size: var(--fs-xs); color: var(--muted); padding: 0 4px; }
      .checks { display: flex; flex-wrap: wrap; gap: 6px 16px; }
      .rules-head { display: flex; align-items: center; gap: 10px; }
      .spacer { flex: 1; }
      .small { font-size: var(--fs-xs); }
      .rules { display: flex; flex-direction: column; gap: 10px; max-height: 46vh; overflow: auto; }
      .rule { border: 1px solid var(--border); border-radius: 6px; padding: 8px 10px; margin: 0; display: flex; flex-direction: column; gap: 8px; }
      .rule .num { flex: 0 0 96px; min-width: 96px; }
      .rule .name { flex: 2; }
      .rm { align-self: flex-end; }
      .invalid { border-color: var(--danger-text) !important; }
      label.invalid input { border-color: var(--danger-text); }
      .ok { color: var(--ok); font-size: var(--fs-sm); }
      .sr { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
    `,
  ],
})
export class AlertRulesEditor implements OnInit {
  static readonly dialogId = 'alert-rules';
  protected readonly ref = inject(DIALOG_REF);
  private readonly profile = inject(LiveRulesProfile);

  protected readonly detectors = DETECTORS;
  protected readonly detectorLabels = DETECTOR_LABELS;
  protected readonly severities = SEVERITIES;
  protected readonly sevLabel = severityLabel;

  protected readonly draft = signal<LiveSettingsDraft | null>(null);
  protected readonly loadError = signal<string | null>(null);
  protected readonly phase = signal<Phase>('loading');
  protected readonly failure = signal<RulesFailure | null>(null);
  protected readonly validated = signal(false);

  private readonly target = computed<ErrorTarget | null>(() => {
    const f = this.failure();
    const d = this.draft();
    return f && d ? errorTarget(f.message, d.rules) : null;
  });

  /** Errors not attached to a rule or a detector checkbox (shown under the timing fields or at the bottom). */
  protected readonly fieldError = computed(() => {
    const t = this.target();
    return t?.kind === 'field' && t.field !== 'alerts' ? this.failure()!.message : null;
  });
  protected readonly generalError = computed(() => (this.target()?.kind === 'general' ? this.failure() : null));

  async ngOnInit(): Promise<void> {
    try {
      this.draft.set(await this.profile.load());
      this.phase.set('ready');
    } catch (e) {
      this.loadError.set(e instanceof Error ? e.message : String(e));
    }
  }

  protected removeLabel(name: string): string {
    return $localize`:@@live.rulesEditor.removeRule:Remove rule ${name}:name:`;
  }

  protected conditionLabel(i: number): string {
    return $localize`:@@live.rulesEditor.condition:Condition of rule ${i + 1}:number:`;
  }

  protected isRule(i: number): boolean {
    const t = this.target();
    return t?.kind === 'rule' && t.index === i;
  }

  protected isField(field: string): boolean {
    const t = this.target();
    return t?.kind === 'field' && t.field === field;
  }

  protected patch(p: Partial<LiveSettingsDraft>): void {
    this.update((d) => ({ ...d, ...p }));
  }

  protected toggleDetector(k: Detector, on: boolean): void {
    this.update((d) => ({ ...d, detectors: on ? [...new Set([...d.detectors, k])] : d.detectors.filter((x) => x !== k) }));
  }

  protected patchRule(i: number, p: Partial<LiveRuleDraft>): void {
    this.update((d) => ({ ...d, rules: d.rules.map((r, j) => (j === i ? { ...r, ...p } : r)) }));
  }

  protected addRule(): void {
    this.update((d) => ({ ...d, rules: [...d.rules, newRule(d.rules.length)] }));
  }

  protected removeRule(i: number): void {
    this.update((d) => ({ ...d, rules: d.rules.filter((_, j) => j !== i) }));
  }

  protected async validate(): Promise<void> {
    const d = this.draft();
    if (!d) return;
    this.phase.set('busy');
    const result = await this.profile.validate(d);
    this.phase.set('ready');
    this.failure.set(result.ok ? null : result.failure);
    this.validated.set(result.ok);
  }

  protected async save(): Promise<void> {
    const d = this.draft();
    if (!d) return;
    this.phase.set('busy');
    const result = await this.profile.save(d);
    this.phase.set('ready');
    if (result.ok) {
      this.ref.close(true);
      return;
    }
    this.failure.set(result.failure);
    this.validated.set(false);
  }

  private update(fn: (d: LiveSettingsDraft) => LiveSettingsDraft): void {
    const d = this.draft();
    if (!d) return;
    this.draft.set(fn(d));
    this.validated.set(false);
  }
}

