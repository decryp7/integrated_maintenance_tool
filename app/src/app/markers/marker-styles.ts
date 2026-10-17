import { computed, inject, Injectable } from '@angular/core';
import { CustomIcon, MarkerKind, WorkspaceSummary } from '../core/api.types';
import { WorkspaceStore } from '../core/workspace.store';
import { GlyphStyle } from './markers.types';

/** The presentation fields of a marker kind. */
export type MarkerStyleFields = Pick<MarkerKind, 'icon' | 'badge' | 'shape' | 'description'>;

/** The presentation fields of a marker kind, as the engine sent them. */
export function styleOf(kind: MarkerKind): MarkerStyleFields {
  return { icon: kind.icon, badge: kind.badge, shape: kind.shape, description: kind.description };
}

/** Custom icons of the summary (none before a workspace is loaded). */
export function summaryIcons(summary: WorkspaceSummary | null | undefined): readonly CustomIcon[] {
  return summary?.icons ?? [];
}

/** Glyph style of a kind as the engine reported it. */
export function glyphStyleOf(kind: MarkerKind): GlyphStyle {
  return { kind: kind.kind, color: kind.color, icon: kind.icon, badge: kind.badge, shape: kind.shape };
}

/** Looks marker kinds up in the current workspace summary for the glyphs, legend and tooltips. */
@Injectable({ providedIn: 'root' })
export class MarkerStyles {
  private readonly store = inject(WorkspaceStore);

  readonly kinds = computed(() => this.store.summary()?.markerKinds ?? []);
  readonly icons = computed(() => summaryIcons(this.store.summary()));
  private readonly byKind = computed(() => new Map(this.kinds().map((k) => [k.kind, k])));

  kind(kind: string): MarkerKind | undefined {
    return this.byKind().get(kind);
  }

  /** The style for a kind id; unknown kinds get a neutral style. */
  style(kind: string): GlyphStyle {
    const k = this.byKind().get(kind);
    return k ? glyphStyleOf(k) : { kind };
  }

  description(kind: string): string | null {
    return this.byKind().get(kind)?.description ?? null;
  }
}
