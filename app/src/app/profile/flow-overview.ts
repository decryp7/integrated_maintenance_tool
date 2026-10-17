import { ChangeDetectionStrategy, Component, computed, inject, output } from '@angular/core';
import { formatCount } from '../core/format';
import { WorkspaceStore } from '../core/workspace.store';
import { ProfileDraft } from './profile-draft';

export type FlowStage = 'files' | 'extract' | 'pipeline' | 'marker';

interface Stage {
  id: FlowStage;
  title: string;
  detail: string;
}

/** The ingest flow at a glance: Import -> Extract -> Pipeline -> Markers, with counts; each stage opens its section. */
@Component({
  selector: 'imt-flow-overview',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <ol class="flow" aria-label="How events are processed">
      @for (s of stages(); track s.id; let last = $last) {
        <li class="stage">
          <button type="button" class="node" (click)="open.emit(s.id)" title="Open this section">
            <span class="title">{{ s.title }}</span>
            <span class="detail">{{ s.detail }}</span>
          </button>
          @if (!last) { <span class="arrow" aria-hidden="true">→</span> }
        </li>
      }
    </ol>
  `,
  styles: `
    :host { display: block; flex: none; padding: 8px 16px; border-bottom: 1px solid var(--border); background: var(--bg-1); }
    .flow { list-style: none; margin: 0; padding: 0; display: flex; align-items: center; gap: 4px; flex-wrap: wrap; }
    .stage { display: flex; align-items: center; gap: 4px; }
    .node { display: flex; flex-direction: column; align-items: flex-start; gap: 1px; padding: 4px 12px; border: 1px solid var(--border-2); border-radius: 8px; background: var(--bg-2); color: var(--text); cursor: pointer; text-align: left; }
    .node:hover { background: var(--bg-hover); }
    .title { font-weight: 600; font-size: 12px; }
    .detail { font-size: 11px; color: var(--muted); }
    .arrow { color: var(--faint); padding: 0 4px; }
  `,
})
export class FlowOverview {
  private readonly draft = inject(ProfileDraft);
  private readonly store = inject(WorkspaceStore);
  readonly open = output<FlowStage>();

  protected readonly stages = computed<Stage[]>(() => {
    const m = this.draft.model();
    const summary = this.store.summary();
    const steps = (m.pipeline ?? []).filter((p) => !p.disabled).length;
    const dropped = summary?.dropped ?? 0;
    const sources = summary?.sources.length ?? 0;
    const plural = (n: number, one: string) => `${formatCount(n)} ${one}${n === 1 ? '' : 's'}`;
    return [
      { id: 'files', title: 'Import', detail: `${plural(sources, 'source')} · ${formatCount((summary?.total ?? 0) + dropped)} events` },
      { id: 'extract', title: 'Extract', detail: plural(m.extract?.length ?? 0, 'extractor') },
      { id: 'pipeline', title: 'Pipeline', detail: `${plural(steps, 'step')}${dropped ? ` · ${formatCount(dropped)} dropped` : ''}` },
      { id: 'marker', title: 'Markers', detail: plural(m.marker?.length ?? 0, 'marker') },
    ];
  });
}
