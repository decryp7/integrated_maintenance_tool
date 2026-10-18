import { unitLabels } from '../core/format';
import { FieldKind, SectionGroup, SettingField, SettingsSection, SettingValue } from './settings.types';

/** Navigation id of the UI-only preferences (theme, density, ...), which are not engine settings. */
export const GENERAL_ID = '@general';

/** Navigation order and titles of the groups the engine assigns. */
export const GROUPS: { id: SectionGroup; title: string }[] = [
  { id: 'app', title: $localize`:Settings group@@settings.group.app:Application` },
  { id: 'diagnostics', title: $localize`:Settings group@@settings.group.diagnostics:Diagnostics` },
  { id: 'plugins', title: $localize`:Settings group@@settings.group.plugins:Plugins` },
  { id: 'ai', title: $localize`:Settings group@@settings.group.ai:AI` },
  { id: 'advanced', title: $localize`:Settings group@@settings.group.advanced:Advanced` },
];

const UNITS_MS: [number, string][] = [
  [3_600_000, unitLabels().h],
  [60_000, unitLabels().min],
  [1000, unitLabels().s],
];
const UNITS_BYTES: [number, string][] = [
  [1 << 30, $localize`:Gigabytes (binary)@@settings.unit.gb:GB`],
  [1 << 20, $localize`:Megabytes (binary)@@settings.unit.mb:MB`],
  [1 << 10, $localize`:Kilobytes (binary)@@settings.unit.kb:KB`],
];
const BYTES = $localize`:Unit after a byte count@@settings.unit.bytes:bytes`;

function largest(n: number, units: [number, string][], fallback: string): string {
  const u = units.find(([size]) => n >= size && n % size === 0);
  return u ? `${n / u[0]} ${u[1]}` : `${n} ${fallback}`;
}

/** `90000` → `"90 s"` (display only; the engine stores milliseconds). */
export function formatMs(ms: number): string {
  return largest(ms, UNITS_MS, unitLabels().ms);
}

/** `10485760` → `"10 MB"` (binary units). */
export function formatBytes(bytes: number): string {
  return largest(bytes, UNITS_BYTES, BYTES);
}

/** The unit a duration or size is edited in: the largest one both limits are whole multiples of. */
export interface EditUnit {
  factor: number;
  label: string;
}

export function editUnit(kind: FieldKind): EditUnit | null {
  const pick = (min: number, max: number, units: [number, string][], base: string): EditUnit => {
    const u = units.find(([size]) => min % size === 0 && max % size === 0 && min >= size);
    return u ? { factor: u[0], label: u[1] } : { factor: 1, label: base };
  };
  if (kind.type === 'duration') return pick(kind.minMs, kind.maxMs, UNITS_MS.slice(2), unitLabels().ms);
  if (kind.type === 'size') return pick(kind.minBytes, kind.maxBytes, UNITS_BYTES, BYTES);
  return null;
}

/** Limits shown next to a control ("1–100 MB", "5 s – 1 h"). */
export function rangeHint(kind: FieldKind): string {
  const unit = (u: string | null) => (u ? ` ${u}` : '');
  switch (kind.type) {
    case 'int':
    case 'float':
      return `${kind.min}–${kind.max}${unit(kind.unit)}`;
    case 'duration':
      return `${formatMs(kind.minMs)} – ${formatMs(kind.maxMs)}`;
    case 'size':
      return `${formatBytes(kind.minBytes)} – ${formatBytes(kind.maxBytes)}`;
    case 'text':
      return $localize`:@@settings.hint.maxLen:up to ${kind.maxLen}:max: characters`;
    case 'stringList':
      return $localize`:@@settings.hint.maxItems:up to ${kind.maxItems}:max: items, one per line`;
    case 'path':
      return kind.dir ? $localize`:Hint next to a path setting@@settings.hint.folder:folder` : $localize`:Hint next to a path setting@@settings.hint.file:file`;
    default:
      return '';
  }
}

/** A value as the user reads it (for "Default: …" hints). */
export function displayValue(field: SettingField, value: SettingValue): string {
  const k = field.kind;
  if (k.type === 'bool') return value ? $localize`:Value of a switched-on setting@@settings.value.on:on` : $localize`:Value of a switched-off setting@@settings.value.off:off`;
  if (k.type === 'duration' && typeof value === 'number') return formatMs(value);
  if (k.type === 'size' && typeof value === 'number') return formatBytes(value);
  if (k.type === 'enum') return k.options.find((o) => o.value === value)?.label ?? String(value);
  if (Array.isArray(value)) return value.length ? value.join(', ') : $localize`:Value of an empty list setting@@settings.value.none:none`;
  if ((k.type === 'int' || k.type === 'float') && k.unit) return `${value} ${k.unit}`;
  return value === '' ? $localize`:Value of an empty setting@@settings.value.notSet:not set` : String(value);
}

export function sameValue(a: unknown, b: unknown): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** Search: every word must appear in the field's label, description or key, or the section title. */
export function fieldMatches(section: SettingsSection, field: SettingField, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  const haystack = `${field.label} ${field.description} ${field.key} ${section.title}`.toLowerCase();
  return words.every((w) => haystack.includes(w));
}

/**
 * Early, convenience-only feedback while typing (ranges and types). The engine validates again on
 * Apply and its messages are the ones that count.
 */
export function precheck(field: SettingField, value: unknown): string | null {
  const k = field.kind;
  const num = typeof value === 'number' && Number.isFinite(value) ? value : null;
  switch (k.type) {
    case 'int':
    case 'float':
      if (num === null) return $localize`:@@settings.check.number:Enter a number.`;
      return num < k.min || num > k.max ? $localize`:@@settings.check.between:Between ${rangeHint(k)}:range:.` : null;
    case 'duration':
    case 'size': {
      const [min, max] = k.type === 'duration' ? [k.minMs, k.maxMs] : [k.minBytes, k.maxBytes];
      if (num === null) return $localize`:@@settings.check.number:Enter a number.`;
      return num < min || num > max ? $localize`:@@settings.check.between:Between ${rangeHint(k)}:range:.` : null;
    }
    case 'text':
      return typeof value === 'string' && [...value].length > k.maxLen ? $localize`:@@settings.check.maxLen:At most ${k.maxLen}:max: characters.` : null;
    case 'stringList':
      return Array.isArray(value) && value.length > k.maxItems ? $localize`:@@settings.check.maxItems:At most ${k.maxItems}:max: items.` : null;
    default:
      return null;
  }
}

/** Parses what was typed into a number input; anything else is passed on as text for the engine to reject. */
export function parseNumber(text: string, factor = 1): number | string {
  const trimmed = text.trim();
  if (trimmed === '') return text;
  const n = Number(trimmed);
  return Number.isFinite(n) ? Math.round(n * factor * 1e6) / 1e6 : text;
}

/** One item per line, blank lines dropped. */
export function parseList(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((l) => l.trim())
    .filter(Boolean);
}

/** Short label of a section's status for the navigation. */
export function statusLabel(section: SettingsSection): string | null {
  switch (section.status) {
    case 'disabled':
      return $localize`:Settings section status@@settings.status.disabled:disabled`;
    case 'quarantined':
      return $localize`:Settings section status (plugin quarantined)@@settings.status.stopped:stopped`;
    case 'notInstalled':
      return $localize`:Settings section status@@settings.status.notInstalled:not installed`;
    case 'safeMode':
      return $localize`:Settings section status@@settings.status.safeMode:safe mode`;
    default:
      return null;
  }
}
