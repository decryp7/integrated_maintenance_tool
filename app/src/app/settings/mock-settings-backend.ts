import { FILE_EXISTS_SUFFIX } from '../core/api.types';
import type { SettingsBackend } from './settings-backend';
import { FieldError, FieldKind, ImportResult, SetResult, SettingField, SettingsOverview, SettingsSection, SettingValue } from './settings.types';
import { formatBytes, formatMs } from './settings-model';

const f = (key: string, label: string, kind: FieldKind, def: SettingValue, extra: Partial<SettingField> = {}): SettingField => ({
  key,
  label,
  description: '',
  kind,
  default: def,
  restartRequired: false,
  advanced: false,
  ...extra,
});
const int = (min: number, max: number, unit: string | null = null): FieldKind => ({ type: 'int', min, max, step: 1, unit });

/** Plausible sections shaped like the engine's built-ins and plugin sections. Browser development and UI tests only. */
export function mockSections(): SettingsSection[] {
  const section = (s: Omit<SettingsSection, 'values' | 'status' | 'readOnly' | 'note'> & Partial<SettingsSection>): SettingsSection => ({
    values: Object.fromEntries(s.fields.map((x) => [x.key, x.default])),
    status: 'active',
    readOnly: false,
    note: null,
    ...s,
  });
  const options = (...o: [string, string][]): FieldKind => ({ type: 'enum', options: o.map(([value, label]) => ({ value, label })) });
  return [
    section({
      id: 'window',
      title: 'Window & tray',
      description: 'Windows, the system tray and what happens when IMT is closed.',
      owner: { kind: 'app' },
      group: 'app',
      fields: [
        f('openFilesIn', 'Open files from other apps in', options(['newWindow', 'A new window'], ['currentWindow', 'The current window'], ['ask', 'Ask every time']), 'newWindow', {
          description: 'Where files opened from Explorer, Finder or another app appear.',
        }),
        f('onClose', 'When the last window is closed', options(['ask', 'Ask'], ['quit', 'Quit IMT'], ['tray', 'Keep running in the tray']), 'ask', {
          description: 'Keeping IMT in the tray lets live tail and monitors continue in the background.',
        }),
        f('notifications', 'Notifications', options(['off', 'Off'], ['alerts', 'Alerts only'], ['all', 'All']), 'alerts', {
          description: 'System notifications while IMT runs in the background.',
        }),
        f('startMinimized', 'Start in the system tray', { type: 'bool' }, false, { description: 'Start IMT without opening a window.' }),
      ],
    }),
    section({
      id: 'diagnostics',
      title: 'Diagnostics',
      description: "IMT's own log files and the screenshots taken when something fails. Nothing is sent anywhere.",
      owner: { kind: 'app' },
      group: 'diagnostics',
      fields: [
        f('logLevel', 'Log level', { type: 'enum', options: ['error', 'warn', 'info', 'debug', 'trace'].map((v) => ({ value: v, label: v[0].toUpperCase() + v.slice(1) })) }, 'info', {
          description: 'How much IMT writes to its own log. Debug and Trace help when reporting a problem.',
        }),
        f('logMaxFileMb', 'Log file size', int(1, 100, 'MB'), 10, { description: 'A new log file is started when the current one reaches this size.', restartRequired: true }),
        f('logKeepFiles', 'Old log files kept', int(1, 50), 5, { description: 'How many rotated log files are kept besides the current one.', restartRequired: true }),
        f('logMaxAgeDays', 'Delete logs older than', int(0, 3650, 'days'), 30, { description: 'Old log files are deleted at startup. 0 keeps them regardless of age.', restartRequired: true, advanced: true }),
        f('activityDetail', 'Activity log detail', { type: 'enum', options: [{ value: 'normal', label: 'Normal' }, { value: 'detailed', label: 'Detailed' }] }, 'normal', {
          description:
            'What the activity log records about your operations. Normal keeps only a fingerprint of query text and the file name of paths; Detailed records full query text and file paths. File contents, log lines and AI text are never recorded.',
        }),
        f('activityQueriesAtDebug', 'Log read-only requests at Debug', { type: 'bool' }, true, {
          description: 'Frequent read-only requests (pages, timeline, charts) are logged only at the Debug level.',
          advanced: true,
        }),
        f('snapshotsEnabled', 'Error snapshots', { type: 'bool' }, true, { description: 'Save a screenshot of IMT locally when an error is shown, to attach to a bug report.' }),
        f('snapshotsMaxCount', 'Snapshots kept', int(1, 500), 50, { description: 'The newest snapshots are kept; older ones are deleted.' }),
        f('snapshotsMaxTotalMb', 'Snapshot storage', int(10, 2000, 'MB'), 100, { description: 'Total disk space snapshots may use.', advanced: true }),
        f('snapshotsMaxAgeDays', 'Delete snapshots older than', int(0, 3650, 'days'), 30, { description: '0 keeps snapshots regardless of age.', advanced: true }),
      ],
    }),
    section({
      id: 'plugins',
      title: 'Plugins',
      description: 'Turn engine plugins (file formats, exporters, tools) on or off. Changes take effect after a restart.',
      owner: { kind: 'app' },
      group: 'plugins',
      fields: [
        ['net.decryptology.registry', 'Windows registry'],
        ['net.decryptology.windows', 'Windows diagnostics'],
        ['net.decryptology.web', 'Web server logs'],
        ['com.example.demo', 'Example plugin'],
      ].map(([id, name]) => f(id, name, { type: 'bool' }, true, { description: `${id} 0.1.0`, restartRequired: true })),
      values: { 'net.decryptology.registry': true, 'net.decryptology.windows': true, 'net.decryptology.web': true, 'com.example.demo': false },
    }),
    section({
      id: 'workspace',
      title: 'Workspace',
      description: 'How IMT follows files that change while they are open.',
      owner: { kind: 'app' },
      group: 'advanced',
      fields: [
        f('liveTailDebounce', 'Live tail delay', { type: 'duration', minMs: 100, maxMs: 10_000 }, 750, {
          description: 'Changes arriving within this time are re-imported together. Takes effect the next time live tail is turned on.',
        }),
      ],
    }),
    section({
      id: 'net.decryptology.windows',
      title: 'Windows diagnostics',
      description: 'External Windows tools used to read Performance Monitor logs and control Data Collector Sets.',
      owner: { kind: 'plugin', id: 'net.decryptology.windows' },
      group: 'plugins',
      fields: [
        f('relogTimeout', 'relog timeout', { type: 'duration', minMs: 5000, maxMs: 3_600_000 }, 120_000, {
          description: 'How long converting one .blg file with relog may take before it is stopped. Raise it for very large logs.',
        }),
        f('logmanTimeout', 'logman timeout', { type: 'duration', minMs: 5000, maxMs: 600_000 }, 120_000, {
          description: 'How long one logman call (import, start, stop, query a Data Collector Set) may take.',
          advanced: true,
        }),
      ],
    }),
    section({
      id: 'com.example.demo',
      title: 'Example plugin',
      description: 'A plugin showing every kind of setting.',
      owner: { kind: 'plugin', id: 'com.example.demo' },
      group: 'plugins',
      status: 'disabled',
      readOnly: true,
      note: 'This plugin is disabled. Enable it under Plugins and restart to change its settings.',
      fields: [
        f('ratio', 'Sample ratio', { type: 'float', min: 0, max: 1, step: 0.05, unit: null }, 0.5),
        f('prefix', 'Prefix', { type: 'text', maxLen: 16, pattern: '[A-Z]+' }, 'IMT'),
        f('outDir', 'Output folder', { type: 'path', dir: true }, ''),
        f('cache', 'Cache size', { type: 'size', minBytes: 1 << 20, maxBytes: 1 << 30 }, 64 << 20),
        f('hosts', 'Hosts', { type: 'stringList', maxItems: 10 }, []),
      ],
    }),
    section({
      id: 'ai',
      title: 'AI assistant',
      description: 'Local AI features: summaries, explanations and query suggestions.',
      owner: { kind: 'app' },
      group: 'ai',
      fields: [],
      status: 'notInstalled',
      readOnly: true,
      note: 'The AI assistant is not installed in this build.',
    }),
  ];
}

/** Mimics the engine's messages closely enough for UI development; the engine is the reference. */
export function mockValidate(field: SettingField, value: unknown): string | null {
  const k = field.kind;
  const num = typeof value === 'number' && Number.isFinite(value) ? value : null;
  const whole = num !== null && Number.isInteger(num) ? num : null;
  const unit = (n: number, u: string | null) => (u ? `${n} ${u}` : `${n}`);
  switch (k.type) {
    case 'bool':
      return typeof value === 'boolean' ? null : 'Must be on or off (true or false).';
    case 'int':
      if (whole === null) return 'Must be a whole number.';
      if (whole < k.min || whole > k.max) return `Must be between ${unit(k.min, k.unit)} and ${unit(k.max, k.unit)}.`;
      return (whole - k.min) % k.step === 0 ? null : `Must be ${k.min} plus a multiple of ${k.step}.`;
    case 'float':
      if (num === null) return 'Must be a number.';
      return num < k.min || num > k.max ? `Must be between ${unit(k.min, k.unit)} and ${unit(k.max, k.unit)}.` : null;
    case 'text':
      if (typeof value !== 'string') return 'Must be text.';
      if ([...value].length > k.maxLen) return `Must be at most ${k.maxLen} characters.`;
      return k.pattern && !new RegExp(`^(?:${k.pattern})$`).test(value) ? `Does not have the expected format (${k.pattern}).` : null;
    case 'enum':
      return typeof value === 'string' && k.options.some((o) => o.value === value) ? null : `Must be one of: ${k.options.map((o) => o.value).join(', ')}.`;
    case 'path':
      if (typeof value !== 'string') return 'Must be a path.';
      return value === '' || /^(\/|[A-Za-z]:\\)/.test(value) ? null : 'Must be an absolute path (or empty).';
    case 'duration':
      if (whole === null || whole < 0) return 'Must be a duration in whole milliseconds.';
      return whole < k.minMs || whole > k.maxMs ? `Must be between ${formatMs(k.minMs)} and ${formatMs(k.maxMs)}.` : null;
    case 'size':
      if (whole === null || whole < 0) return 'Must be a size in whole bytes.';
      return whole < k.minBytes || whole > k.maxBytes ? `Must be between ${formatBytes(k.minBytes)} and ${formatBytes(k.maxBytes)}.` : null;
    case 'stringList':
      if (!Array.isArray(value)) return 'Must be a list of text.';
      if (value.length > k.maxItems) return `Must have at most ${k.maxItems} items.`;
      return value.every((v) => typeof v === 'string') ? null : 'Items must be text.';
  }
}

/** In-memory settings for the browser build. `fail` makes every call reject (error-path tests). */
export class MockSettingsBackend implements SettingsBackend {
  private readonly data: SettingsSection[] = mockSections();
  private restartPending = false;
  /** Files "written" by export, by path. */
  readonly files = new Map<string, string>();
  readonly calls: string[] = [];

  constructor(private readonly options: { fail?: boolean } = {}) {}

  private async guard(call: string): Promise<void> {
    this.calls.push(call);
    if (this.options.fail) throw 'the settings service is unavailable';
  }

  async sections(): Promise<SettingsOverview> {
    await this.guard('sections');
    return structuredClone({ sections: this.data, restartPending: this.restartPending, safeMode: false });
  }

  async set(id: string, values: Record<string, SettingValue | null>): Promise<SetResult> {
    await this.guard(`set:${id}`);
    const section = this.data.find((s) => s.id === id);
    if (!section) throw `there is no settings section "${id}"`;
    if (section.readOnly) throw section.note ?? 'This section is read-only.';
    const next: Record<string, SettingValue> = {};
    const errors: FieldError[] = [];
    for (const field of section.fields) {
      const v = values[field.key];
      if (v === undefined || v === null) {
        next[field.key] = field.default;
        continue;
      }
      const message = mockValidate(field, v);
      if (message) errors.push({ key: field.key, message });
      next[field.key] = v;
    }
    if (errors.length) return { applied: false, restartRequired: false, errors, values: structuredClone(section.values) };
    const restartRequired = section.fields.some((x) => x.restartRequired && JSON.stringify(section.values[x.key]) !== JSON.stringify(next[x.key]));
    section.values = next;
    this.restartPending ||= restartRequired;
    return { applied: true, restartRequired, errors: [], values: structuredClone(next) };
  }

  reset(id: string): Promise<SetResult> {
    return this.set(id, {});
  }

  async exportTo(path: string, overwrite = false): Promise<void> {
    await this.guard(`export:${path}`);
    if (!overwrite && this.files.has(path)) throw `${path}${FILE_EXISTS_SUFFIX}`;
    this.files.set(path, JSON.stringify({ version: 1, sections: Object.fromEntries(this.data.map((s) => [s.id, s.values])) }));
  }

  async importFrom(path: string): Promise<ImportResult> {
    await this.guard(`import:${path}`);
    const text = this.files.get(path);
    if (text === undefined) throw `cannot open ${path}: no such file`;
    const parsed = JSON.parse(text) as { sections: Record<string, Record<string, SettingValue>> };
    const result: ImportResult = { applied: [], kept: [], rejected: [], restartRequired: false };
    for (const [id, values] of Object.entries(parsed.sections)) {
      const section = this.data.find((s) => s.id === id);
      if (!section || section.readOnly) {
        result.kept.push(id);
        continue;
      }
      const r = await this.set(id, values);
      if (r.applied) {
        result.applied.push(id);
        result.restartRequired ||= r.restartRequired;
      } else result.rejected.push({ section: id, errors: r.errors });
    }
    return result;
  }
}
