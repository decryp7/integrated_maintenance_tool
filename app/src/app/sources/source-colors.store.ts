import { computed, inject, Injectable } from '@angular/core';
import { Lane, RecordView, SourceInfo } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { SettingsStore } from '../settings/settings.store';
import { InstanceSlots, Swatch, swatchAt } from './source-colors';

/** A swatch with what it stands for (the legend text of its tooltip). */
export interface SourceMark {
  swatch: Swatch;
  label: string;
}

/** Most source dots a timeline lane label shows (the tooltip lists every one). */
export const LANE_DOTS = 3;

/**
 * The colour marker of sources or instances, following the user's choice (Settings › General, View › Source colours).
 * Colours are stable: a source's comes from its engine id, an instance's from the order it was first seen.
 */
@Injectable({ providedIn: 'root' })
export class SourceColors {
  private readonly settings = inject(SettingsStore);
  private readonly store = inject(WorkspaceStore);
  private readonly instances = new InstanceSlots();
  readonly mode = this.settings.sourceColors;
  readonly enabled = computed(() => this.mode() !== 'off');
  private readonly sources = computed(() => new Map((this.store.summary()?.sources ?? []).map((s) => [s.id, s])));

  /** The marker of source `id` (shown whenever markers are on: the Explorer and inspector always name files). */
  forSource(id: number): SourceMark | null {
    if (!this.enabled()) return null;
    const name = this.sources().get(id)?.name ?? `#${id}`;
    return { swatch: swatchAt(id), label: $localize`:Legend of a source colour@@sources.colors.legend.source:File: ${name}:name:` };
  }

  /** The marker of an instance key (only in "by instance" mode). */
  forInstance(key: string): SourceMark | null {
    if (this.mode() !== 'instance') return null;
    return { swatch: swatchAt(this.instances.slotOf(key)), label: $localize`:Legend of an instance colour@@sources.colors.legend.instance:Instance: ${key}:key:` };
  }

  /** The marker of a log line: its file or its instance, as chosen. */
  forRecord(r: RecordView): SourceMark | null {
    return this.mode() === 'instance' ? this.forInstance(r.group) : this.forSource(r.source);
  }

  /** The marker of an Explorer row: the file's, in "by file" mode. */
  forExplorer(s: SourceInfo): SourceMark | null {
    return this.mode() === 'source' ? this.forSource(s.id) : null;
  }

  /** The markers of a timeline lane: the instance's, or its files' (at most {@link LANE_DOTS}). */
  forLane(lane: Lane): SourceMark[] {
    if (this.mode() === 'instance') return [this.forInstance(lane.key)].filter((m): m is SourceMark => !!m);
    return (lane.sources ?? [])
      .slice(0, LANE_DOTS)
      .map((id) => this.forSource(id))
      .filter((m): m is SourceMark => !!m);
  }
}
