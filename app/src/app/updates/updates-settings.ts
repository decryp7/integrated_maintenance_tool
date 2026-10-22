import { ChangeDetectionStrategy, Component, computed, inject } from '@angular/core';
import { activeLocale } from '../i18n/locale';
import { UpdatesCommands } from './updates.commands';
import { UpdatesStore } from './updates.store';

/** "Last checked 20 Oct 2026, 14:05" or "Never checked". */
export function lastCheckedText(iso: string | null): string {
  const d = iso ? new Date(iso) : null;
  if (!d || Number.isNaN(d.getTime())) return $localize`:@@settings.updates.neverChecked:Never checked`;
  const when = d.toLocaleString(activeLocale(), { dateStyle: 'medium', timeStyle: 'short' });
  return $localize`:@@settings.updates.lastChecked:Last checked ${when}:when:`;
}

/**
 * Settings ▸ Updates: the version, the app's update state and schedule (automatic checks), and where the AI runtime
 * and model updates are. The flow itself (check, download, install) stays in the Updates dialog.
 */
@Component({
  selector: 'debrief-updates-settings',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <h3 i18n="Settings section|Application and AI updates@@settings.updates.nav">Updates</h3>
    <p class="desc" i18n="@@settings.updates.desc">Debrief checks for new versions by itself and never installs one without asking.</p>
    <dl class="facts" data-testid="updates-facts">
      <dt i18n="@@settings.updates.version">Version</dt>
      <dd>{{ store.status()?.currentVersion ?? '…' }}</dd>
      <dt i18n="@@settings.updates.state">State</dt>
      <dd data-testid="updates-state">{{ stateText() }}</dd>
      <dt i18n="@@settings.updates.schedule">Schedule</dt>
      <dd>{{ scheduleText() }} · {{ lastChecked() }}</dd>
    </dl>
    <div class="actions">
      <button type="button" class="btn primary" data-testid="updates-check" (click)="commands.open()" i18n="@@settings.updates.check">Check for updates…</button>
    </div>
    <p class="desc" data-testid="updates-ai-note" i18n="@@settings.updates.aiNote">Updates of the AI runtime and its models are managed in the AI section of Settings.</p>
  `,
  styles: `
    :host { display: block; }
    h3 { margin: 0 0 4px; font-size: var(--fs-lg); font-weight: 600; }
    .desc { margin: 4px 0 12px; color: var(--muted); }
    .facts { display: grid; grid-template-columns: max-content 1fr; gap: 6px 16px; margin: 0 0 14px; }
    dt { color: var(--muted); }
    dd { margin: 0; }
    .actions { display: flex; gap: 8px; flex-wrap: wrap; }
  `,
})
export class UpdatesSettings {
  protected readonly store = inject(UpdatesStore);
  protected readonly commands = inject(UpdatesCommands);
  protected readonly lastChecked = computed(() => lastCheckedText(this.store.status()?.lastChecked ?? null));
  protected readonly scheduleText = computed(() =>
    this.store.status()?.autoCheck ? $localize`:@@settings.updates.auto:Checks automatically` : $localize`:@@settings.updates.manual:Checks only when you ask`,
  );
  protected readonly stateText = computed(() => {
    const p = this.store.phase();
    switch (p?.kind) {
      case 'unavailable':
        return $localize`:@@settings.updates.state.unavailable:Not available in this build`;
      case 'checking':
        return $localize`:@@settings.updates.state.checking:Checking…`;
      case 'upToDate':
        return $localize`:@@settings.updates.state.upToDate:Up to date`;
      case 'available':
        return $localize`:@@settings.updates.state.available:Version ${p.info.version}:version: is available`;
      case 'downloading':
        return $localize`:@@settings.updates.state.downloading:Downloading version ${p.info.version}:version:`;
      case 'ready':
        return $localize`:@@settings.updates.state.ready:Version ${p.info.version}:version: is ready to install`;
      case 'installing':
        return $localize`:@@settings.updates.state.installing:Installing…`;
      case 'failed':
        return $localize`:@@settings.updates.state.failed:The last check failed`;
      default:
        return $localize`:@@settings.updates.state.idle:No check yet in this run`;
    }
  });

  constructor() {
    void this.store.refresh();
  }
}
