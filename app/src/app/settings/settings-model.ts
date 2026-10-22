import { unitLabels } from '../core/format';
import { VIDEO_SETTINGS_ID } from '../video/video-settings.types';
import { KEYBOARD_ID } from './keyboard/keyboard.types';
import { PARSERS_ID } from './parsers/parsers.types';
import { STORAGE_ID } from './storage/storage.types';
import { FieldKind, SectionGroup, SettingField, SettingsSection, SettingValue } from './settings.types';

/** Navigation id of the UI-only preferences (theme, density, ...), which are not engine settings. */
export const GENERAL_ID = '@general';
/** Navigation id of Appearance: theme, colour scheme, density (UI preferences, not engine settings). */
export const APPEARANCE_ID = '@appearance';

/** Navigation id of Updates: app updates, AI runtime and model updates (UI-only; the engine owns the updater). */
export const UPDATES_ID = '@updates';
/** The engine section that holds the live tail delay; the Live tail page shows it with the rest of live tail. */
export const LIVE_TAIL_SECTION = 'workspace';

/** One entry of the navigation: a UI page (`ui`) or an engine section. */
export interface NavItem {
  id: string;
  title: string;
  /** The engine section behind the entry, when it is one. */
  section?: SettingsSection;
}

export interface NavGroup {
  id: 'application' | 'data' | 'ai' | 'plugins' | 'diagnostics';
  title: string;
  items: NavItem[];
}

/**
 * The navigation: Application (General, Appearance, window, Updates, Disk usage, performance), Data (Parsers, Live
 * tail), AI, Plugins (one entry per plugin) and Diagnostics. UI pages and the engine's sections are placed by id and
 * group; a section the layout does not know falls into Application, so nothing is unreachable.
 */
export function navLayout(sections: readonly SettingsSection[]): NavGroup[] {
  const titles = {
    general: $localize`:Settings section|Name of the first settings section@@settings.screen.general:General`,
    appearance: $localize`:Settings section|Theme, colour scheme and density@@settings.screen.appearance:Appearance`,
    updates: $localize`:Settings section|Application and AI updates@@settings.updates.nav:Updates`,
    storage: $localize`:Settings section|What IMT keeps on disk@@settings.storage.nav:Disk usage`,
    parsers: $localize`:Settings section|The log formats IMT reads@@settings.parsers.nav:Parsers`,
    keyboard: $localize`:Settings section|Key bindings of the commands@@settings.keyboard.nav:Keyboard shortcuts`,
    video: $localize`:Settings section|Videos and the FFmpeg runtime@@video.settings.nav:Video`,
    liveTail: $localize`:Settings section|Live tail options@@settings.liveTail.nav:Live tail`,
  };
  const item = (s: SettingsSection): NavItem => ({ id: s.id, title: s.id === LIVE_TAIL_SECTION ? titles.liveTail : s.title, section: s });
  const by = (pred: (s: SettingsSection) => boolean) => sections.filter(pred).map(item);
  const inGroup = (...g: SectionGroup[]) => (s: SettingsSection) => g.includes(s.group) && s.id !== LIVE_TAIL_SECTION;
  const groups: NavGroup[] = [
    {
      id: 'application',
      title: $localize`:Settings group@@settings.group.app:Application`,
      items: [
        { id: GENERAL_ID, title: titles.general },
        { id: APPEARANCE_ID, title: titles.appearance },
        { id: KEYBOARD_ID, title: titles.keyboard },
        ...by(inGroup('app')),
        { id: UPDATES_ID, title: titles.updates },
        { id: STORAGE_ID, title: titles.storage },
        ...by(inGroup('performance', 'advanced')),
      ],
    },
    {
      id: 'data',
      title: $localize`:Settings group@@settings.group.data:Data`,
      items: [{ id: PARSERS_ID, title: titles.parsers }, { id: VIDEO_SETTINGS_ID, title: titles.video }, ...by((s) => s.id === LIVE_TAIL_SECTION), ...by(inGroup('symbols'))],
    },
    { id: 'ai', title: $localize`:Settings group@@settings.group.ai:AI`, items: by(inGroup('ai')) },
    { id: 'plugins', title: $localize`:Settings group@@settings.group.plugins:Plugins`, items: by(inGroup('plugins')) },
    { id: 'diagnostics', title: $localize`:Settings group@@settings.group.diagnostics:Diagnostics`, items: by(inGroup('diagnostics')) },
  ];
  return groups.filter((g) => g.items.length > 0);
}

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
