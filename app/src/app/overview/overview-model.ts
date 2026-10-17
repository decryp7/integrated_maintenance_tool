import { Finding, FindingAction, PluginInfo, Severity, SEVERITIES, UnrecognizedFile, WorkspaceSummary } from '../core/api.types';

/** Findings of one severity, in the engine's order. */
export interface SeverityGroup {
  severity: Severity;
  label: string;
  /** Name of an `<imt-icon>`; the label is always shown too (never colour alone). */
  icon: string;
  findings: Finding[];
}

export const SEVERITY_LABELS: Record<Severity, string> = { critical: 'Critical', warning: 'Warning', info: 'Info' };
const SEVERITY_ICONS: Record<Severity, string> = { critical: 'alert', warning: 'alert', info: 'check' };

/** Groups findings by severity, most severe first; empty groups are left out. */
export function groupBySeverity(findings: readonly Finding[]): SeverityGroup[] {
  return SEVERITIES.map((severity) => ({ severity, label: SEVERITY_LABELS[severity], icon: SEVERITY_ICONS[severity], findings: findings.filter((f) => f.severity === severity) })).filter(
    (g) => g.findings.length > 0,
  );
}

/** A plugin that needs attention, and why in a few words. */
export interface PluginProblem {
  id: string;
  name: string;
  problem: string;
}

/** Quarantined, inactive, failing or warning plugins. */
export function pluginProblems(plugins: readonly PluginInfo[]): PluginProblem[] {
  return plugins.flatMap((p): PluginProblem[] => {
    if (p.status === 'quarantined') return [{ id: p.id, name: p.name, problem: `disabled after crashes${p.lastError ? `: ${p.lastError}` : ''}` }];
    if (p.status === 'inactive') return [{ id: p.id, name: p.name, problem: p.inactiveReason ?? 'not running' }];
    if (p.failures > 0) return [{ id: p.id, name: p.name, problem: `${p.failures} contained error${p.failures === 1 ? '' : 's'}${p.lastError ? `: ${p.lastError}` : ''}` }];
    if (p.warnings.length) return [{ id: p.id, name: p.name, problem: p.warnings[0] }];
    return [];
  });
}

/** The headline numbers of the overview. */
export interface OverviewStats {
  sources: number;
  events: number;
  instances: number;
  from: number | null;
  to: number | null;
  errors: number;
  crashes: number;
  unrecognized: UnrecognizedFile[];
  plugins: PluginProblem[];
}

export function overviewStats(summary: WorkspaceSummary, instances: number, findings: readonly Finding[]): OverviewStats {
  return {
    sources: summary.sources.length,
    events: summary.total,
    instances,
    from: summary.from,
    to: summary.to,
    errors: (summary.levels.Error ?? 0) + (summary.levels.Fatal ?? 0),
    crashes: findings.filter((f) => f.kind === 'crash').length,
    unrecognized: summary.unrecognized,
    plugins: pluginProblems(summary.plugins),
  };
}

/** What a finding's button does. */
export type ActionId = 'jump' | 'showRange' | 'showException' | 'openExceptions' | 'setupFormat' | 'openPanel';

export interface ActionButton {
  id: ActionId;
  label: string;
}

/** Views a finding's `openPanel` may name, mapped to the app's view ids and titles. */
const PANELS: Record<string, { view: string; title: string }> = {
  pipeline: { view: 'profile', title: 'Profile pipeline' },
  plugins: { view: 'plugins', title: 'Plugins' },
};

/** The view an `openPanel` action opens (unknown panels are opened by their own id). */
export function panelTarget(panel: string): { view: string; title: string } {
  return PANELS[panel] ?? { view: panel, title: panel.charAt(0).toUpperCase() + panel.slice(1) };
}

/** The buttons offered for a finding's action. */
export function actionButtons(action: FindingAction | null): ActionButton[] {
  switch (action?.kind) {
    case 'zoom':
      return [
        { id: 'jump', label: 'Jump to time' },
        { id: 'showRange', label: 'Show these events' },
      ];
    case 'exception':
      return [
        { id: 'showException', label: 'Show events' },
        { id: 'openExceptions', label: 'Open Exceptions' },
      ];
    case 'setupFormat':
      return [{ id: 'setupFormat', label: 'Set up format…' }];
    case 'openPanel':
      return [{ id: 'openPanel', label: `Open ${panelTarget(action.panel).title}` }];
    default:
      return [];
  }
}

/** A zoom window around [from, to]: at least a minute, with a margin of a quarter of the span on each side. */
export function zoomWindow(from: number, to: number): { from: number; to: number } {
  const span = Math.max(60_000, to - from);
  const margin = Math.round(span / 4);
  const mid = Math.round((from + to) / 2);
  const half = Math.round(span / 2) + margin;
  return { from: Math.min(from, mid - half), to: Math.max(to, mid + half) };
}
