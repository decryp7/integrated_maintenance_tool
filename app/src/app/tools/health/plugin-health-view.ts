import { ChangeDetectionStrategy, Component, inject } from '@angular/core';
import { ToolError } from '../elevation-hint';
import { eventName, PluginHealthStore, PluginRow } from './plugin-health.store';

/** Plugin health: status, repeat failures and the last error, with reset and enable/disable actions. */
@Component({
  selector: 'imt-plugin-health-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [ToolError],
  template: `
    <div class="scroll" role="region" aria-label="Plugin health" tabindex="0">
      <p class="intro">A plugin that keeps failing is quarantined so it cannot disturb your work. Reset it to try again, or disable it from the next start.</p>
      <imt-tool-error [message]="store.error()" (dismissed)="store.dismissError()" />
      @if (store.needsRestart()) {
        <div class="restart" role="status">
          <span>Plugin changes apply after a restart.</span>
          <button type="button" class="btn primary" (click)="store.restart()">Restart to apply</button>
        </div>
      }
      @if (store.plugins().length) {
        <ul class="list" aria-label="Plugin health">
          @for (p of store.plugins(); track p.id) {
            <li class="plugin" [attr.data-status]="p.status">
              <div class="top">
                <strong>{{ p.name }}</strong> <span class="mono faint">v{{ p.version }}</span>
                <span class="badge" [class]="'st ' + p.status" data-testid="plugin-status">{{ p.status }}</span>
                @if (p.hung) { <span class="badge st hung" data-testid="plugin-hung" title="A call is running past its time budget right now">not responding</span> }
                @if (p.pendingEnabled !== null) { <span class="badge">{{ p.pendingEnabled ? 'will be enabled' : 'will be disabled' }}</span> }
                <span class="mono faint id">{{ p.id }}</span>
              </div>
              @if (p.failures > 0) { <div class="fail">{{ p.failures }} failure{{ p.failures === 1 ? '' : 's' }}</div> }
              @if (p.lastError) { <div class="err mono" [title]="p.lastError">{{ p.lastError }}</div> }
              @if (p.inactiveReason) { <div class="reason" data-testid="inactive-reason">Not running: {{ p.inactiveReason }}</div> }
              @if (p.requires.length || p.provides.length || p.dependents.length) {
                <dl class="deps">
                  @if (p.requires.length) {
                    <dt>Requires</dt>
                    <dd>@for (d of p.requires; track d.id) { <span class="badge mono" [title]="d.optional ? 'Optional dependency' : 'Required dependency'">{{ d.id }} {{ d.req }}{{ d.optional ? ' (optional)' : '' }}</span> }</dd>
                  }
                  @if (p.provides.length) {
                    <dt>Provides</dt>
                    <dd>@for (sv of p.provides; track sv.id) { <span class="badge mono">{{ sv.id }} {{ sv.version }}</span> }</dd>
                  }
                  @if (p.dependents.length) {
                    <dt>Needed by</dt>
                    <dd>@for (d of p.dependents; track d) { <span class="badge mono">{{ d }}</span> } <span class="faint">(they stop when this plugin stops)</span></dd>
                  }
                </dl>
              }
              @if (p.warnings.length) {
                <ul class="warnings" aria-label="Warnings">
                  @for (w of p.warnings; track $index) { <li>{{ w }}</li> }
                </ul>
              }
              @if (store.eventsByPlugin().get(p.id); as subs) {
                <table class="events" [attr.aria-label]="'Events delivered to ' + p.name">
                  <thead><tr><th scope="col">Event</th><th scope="col" class="num">Delivered</th><th scope="col" class="num">Dropped</th><th scope="col" class="num">Failures</th><th scope="col" class="num">Queued</th></tr></thead>
                  <tbody>
                    @for (m of subs; track m.id) {
                      <tr [class.bad]="m.dropped > 0 || m.failures > 0">
                        <td class="mono" [title]="m.event">{{ event(m.event) }}</td>
                        <td class="num mono">{{ m.delivered }}</td>
                        <td class="num mono">{{ m.dropped }}</td>
                        <td class="num mono">{{ m.failures }}</td>
                        <td class="num mono">{{ m.queued }}</td>
                      </tr>
                    }
                  </tbody>
                </table>
              }
              <div class="actions">
                @if (p.status === 'quarantined') {
                  <button type="button" class="btn" [disabled]="!!store.busy()" (click)="store.reset(p.id)">Reset</button>
                }
                @if (isOn(p)) {
                  <button type="button" class="btn" [disabled]="!!store.busy()" (click)="store.setEnabled(p.id, false)">Disable</button>
                } @else {
                  <button type="button" class="btn" [disabled]="!!store.busy()" (click)="store.setEnabled(p.id, true)">Enable</button>
                }
              </div>
            </li>
          }
        </ul>
      } @else {
        <p class="faint">No plugins are loaded.</p>
      }
      <div class="events-bar">
        @if (store.eventsError(); as e) { <span class="err" role="alert">Event counters unavailable: {{ e }}</span> }
        <button type="button" class="btn" (click)="store.loadEvents()">Refresh event counters</button>
      </div>
    </div>
  `,
  styles: `
    :host { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg-1); }
    .scroll { flex: 1; min-height: 0; overflow: auto; padding: 14px 18px; display: flex; flex-direction: column; gap: 10px; max-width: 780px; }
    .scroll:focus-visible { outline-offset: -2px; }
    .intro { margin: 0; color: var(--text-2); line-height: 1.5; }
    .restart { display: flex; align-items: center; gap: 12px; padding: 8px 10px; border: 1px solid var(--accent); border-radius: 6px; background: var(--bg-3); }
    .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 8px; }
    .plugin { padding: 8px 10px; border: 1px solid var(--border-2); border-radius: 8px; background: var(--bg-2); display: flex; flex-direction: column; gap: 6px; }
    .plugin[data-status='quarantined'] { border-color: var(--lvl-error); }
    .top { display: flex; gap: 8px; align-items: baseline; flex-wrap: wrap; }
    .id { margin-left: auto; font-size: 11px; }
    .st.active { color: var(--ok); }
    .st.quarantined { color: var(--danger-text); }
    .st.disabled { color: var(--muted); }
    .st.inactive { color: var(--warn-text); }
    .st.hung { color: var(--danger-text); }
    .plugin[data-status='inactive'] { border-color: var(--lvl-warn); }
    .reason { font-size: 12px; color: var(--warn-text); }
    .deps { display: grid; grid-template-columns: max-content 1fr; gap: 4px 10px; margin: 0; font-size: 12px; }
    .deps dt { color: var(--muted); }
    .deps dd { margin: 0; display: flex; flex-wrap: wrap; gap: 4px; align-items: center; }
    .warnings { margin: 0; padding-left: 18px; font-size: 12px; color: var(--warn-text); }
    .events { border-collapse: collapse; font-size: 11.5px; }
    .events th, .events td { padding: 2px 8px; text-align: left; border-bottom: 1px solid var(--border); }
    .events .num { text-align: right; }
    .events tr.bad td { color: var(--warn-text); }
    .events-bar { display: flex; gap: 10px; align-items: center; }
    .fail { font-size: 12px; color: var(--warn-text); }
    .err { font-size: 11.5px; color: var(--text-2); white-space: pre-wrap; word-break: break-word; max-height: 72px; overflow: auto; }
    .actions { display: flex; gap: 8px; }
  `,
})
export class PluginHealthView {
  protected readonly store = inject(PluginHealthStore);
  protected readonly event = eventName;

  constructor() {
    void this.store.loadEvents();
  }

  /** Whether the plugin will be on after the next start. */
  protected isOn(p: PluginRow): boolean {
    return p.pendingEnabled ?? p.status !== 'disabled';
  }
}
