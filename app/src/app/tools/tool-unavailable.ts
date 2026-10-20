import { ChangeDetectionStrategy, Component, computed, inject, input } from '@angular/core';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { ToolCatalog } from './tool-catalog.store';

/** Says why a tool cannot be used right now (plugin inactive or quarantined, safe mode, plugin disabled). */
@Component({
  selector: 'imt-tool-unavailable',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @if (message(); as text) {
      <div class="unavailable" role="status" data-testid="tool-unavailable">{{ text }}</div>
    }
  `,
  styles: `
    .unavailable { padding: 8px 10px; border-radius: 6px; border: 1px solid var(--lvl-warn); background: var(--bg-3); color: var(--text); font-size: var(--fs-sm); }
  `,
})
export class ToolUnavailable {
  private readonly catalog = inject(ToolCatalog);
  private readonly life = inject(LifecycleService);
  /** The tool id (`TOOL_IDS`). */
  readonly tool = input.required<string>();

  protected readonly message = computed(() => {
    const problem = this.catalog.problem(this.tool());
    if (!problem) return null;
    if (problem.kind === 'plugin') return $localize`:@@tools.unavailable.plugin:This tool is unavailable: ${problem.reason}:reason:.`;
    return this.life.safeMode()
      ? $localize`:@@tools.unavailable.safeMode:This tool is off in safe mode (plugins are not loaded). Restart normally to use it.`
      : $localize`:@@tools.unavailable.missing:This tool is not installed: the System tools plugin is disabled. Enable it in Plugin health and restart.`;
  });
}
