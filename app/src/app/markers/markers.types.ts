/**
 * UI-only types of the marker editor and glyphs. The contract types (`MarkerKind`, `MarkerShape`,
 * `CustomIcon`, ...) live in `core/api.types.ts`.
 */
import { Condition, MarkerRule } from '../core/profile.model';

/** Everything needed to draw one marker glyph. */
export interface GlyphStyle {
  /** Marker kind (selects the built-in shape for the default kinds). */
  kind?: string;
  color?: string | null;
  icon?: string | null;
  badge?: string | null;
  shape?: string | null;
  /** Pre-resolved custom artwork (an unsaved import); wins over `icon` lookups. */
  svg?: string | null;
  pngDataUrl?: string | null;
}

/** A `[[marker]]` rule with the presentation keys. */
export type MarkerRuleX = MarkerRule & {
  icon?: string;
  badge?: string;
  shape?: string;
  description?: string;
};

/** A rule in priority order plus whether it is active (disabled rules live in `imt_disabled_markers`). */
export interface RuleRow {
  rule: MarkerRuleX;
  enabled: boolean;
}

/** Custom icon artwork queued to be written as `[[icon]]` on save. */
export interface PendingIcon {
  name: string;
  /** SVG source text. */
  svg?: string;
  /** Base64 PNG. */
  png?: string;
  /** Data URL for previews of a PNG. */
  pngDataUrl?: string;
}

export interface SaveFailure {
  message: string;
  /** 1-based line in the generated profile TOML, when known. */
  line: number | null;
  /** The offending TOML line, for context. */
  excerpt: string | null;
}

export type ConditionList = Condition[];
