import { activity, isCancellation, Operation } from './runtime';
import { safePath, safeText } from './privacy';
import { ActivityParams } from './types';

/**
 * Activity of backend calls, applied by the shared backend proxy (`withFailureLogging`) to every method of every
 * backend, mock or real. A call made while an operation is current belongs to it (inside Tauri the IPC layer
 * also sends its id to Rust). A call that changes something while no operation is current is a user action
 * nobody instrumented (a dialog's button calling the backend directly): it becomes an operation of its own,
 * named after the method (or the domain operation it is, e.g. `file.export`), with an inferred trigger.
 */

export type MethodKind = 'query' | 'mutation' | 'quiet' | 'event';

/** Read-only method names, by prefix. */
const QUERY_PREFIX =
  /^(get|list|query|preview|read|validate|check|complete|explain|infer|suggest|estimate|status|describe|find|search|load|fetch|count|summary|summar(y|ize)|catalog|recommend|hardware|history|version|templates|formats|is|has|can|sections|elevation|appStatus|lastSession|tail|liveState|workspaceSummary|annotations|pluginEvents|overview|fieldStats|correlate|compare|diff|sniff|automaticEnabled|models|chatSessions|chatGet|chatHistory|suggestedPrompts|perfmonPresets|perfmonOutputs|monitorStatus|eventLog(Channels|Validate|Status)|collectDefaultPlan|toolsList|current|pageOf|windows|popouts|label|demoPaths|workAreas|windowBounds|contentOrigin|hostGone|profiling(?!Open|Close))/;

/** Bookkeeping the UI does on its own (autosave, polling): never an operation. */
const QUIET = new Set(['saveSession', 'takePendingOpen', 'importProgress', 'chatEventsSince', 'setWindowTitle', 'setRecentProjects', 'cancel', 'syncPort', 'setAppMenu']);

/** Methods that look like queries by name but change something. */
const MUTATION = new Set(['previewRedaction']);

export function methodKind(name: string): MethodKind {
  if (/^on[A-Z]/.test(name)) return 'event';
  if (QUIET.has(name)) return 'quiet';
  if (MUTATION.has(name)) return 'mutation';
  if (QUERY_PREFIX.test(name)) return 'query';
  return 'mutation';
}

const detail = () => activity.detail;
const paths = (args: unknown[]) => (Array.isArray(args[0]) ? (args[0] as unknown[]).filter((p): p is string => typeof p === 'string') : []);
const firstPath = (args: unknown[]): ActivityParams => {
  const p = paths(args);
  const [first] = p;
  return first !== undefined ? { Files: p.length, Path: safePath(first, detail()) } : {};
};
const stringArg = (i: number, key: string) => (args: unknown[]) => (typeof args[i] === 'string' ? { [key]: safePath(args[i] as string, detail()) } : {});

/** Domain operations for backend methods (operation name and safe parameters from the arguments). */
const DOMAIN: Record<string, (args: unknown[]) => { operation: string; params: ActivityParams }> = {
  openPaths: (a) => ({ operation: 'file.open', params: firstPath(a) }),
  openPathsLive: (a) => ({ operation: 'file.open', params: { ...firstPath(a), Live: true } }),
  openProject: (a) => ({ operation: 'file.open', params: stringArg(0, 'Path')(a) }),
  openBundle: (a) => ({ operation: 'file.open', params: stringArg(0, 'Path')(a) }),
  saveProject: (a) => ({ operation: 'file.save', params: stringArg(0, 'Path')(a) }),
  exportEvents: (a) => {
    const req = (a[0] ?? {}) as { format?: unknown; path?: unknown };
    return {
      operation: 'file.export',
      params: { Format: typeof req.format === 'string' ? req.format : undefined, Path: typeof req.path === 'string' ? safePath(req.path, detail()) : undefined },
    };
  },
  setLive: (a) => ({ operation: a[0] ? 'live.on' : 'live.off', params: {} }),
  set: (a) => ({
    operation: 'settings.change',
    params: {
      Section: typeof a[0] === 'string' ? a[0] : undefined,
      Fields: a[1] && typeof a[1] === 'object' ? Object.keys(a[1] as object).slice(0, 8).join(',') : undefined,
    },
  }),
  reset: (a) => ({ operation: typeof a[0] === 'string' ? 'settings.reset' : 'backend.reset', params: { Section: typeof a[0] === 'string' ? a[0] : undefined } }),
  chatSend: () => ({ operation: 'ai.ask', params: {} }),
  nlToQuery: (a) => ({ operation: 'ai.ask', params: typeof a[0] === 'string' ? safeText('Question', a[0], 'normal') : {} }),
};

/**
 * Called by the backend proxy before each method call. Returns the operation the call starts (only for a
 * mutation outside any operation), to be ended by `backendCallSettled`; null otherwise.
 */
export function backendCallStarted(method: string, args: unknown[]): Operation | null {
  if (activity.attributable() || methodKind(method) !== 'mutation') return null;
  // No user input led to it (start-up, timers, sync with another window): not a user operation. Rust still
  // logs the command itself.
  const trigger = activity.triggers.infer();
  if (trigger === 'program') return null;
  const domain = DOMAIN[method]?.(args);
  // A call with no domain name is weak: a state change it leads to (a panel popped out) is still logged as the
  // user's operation.
  if (!domain) return activity.begin(`backend.${method}`, trigger, { Method: method }, { weak: true });
  return activity.begin(domain.operation, trigger, { ...domain.params, Method: method });
}

/** Ends the operation `backendCallStarted` began (no-op for null). */
export function backendCallSettled(op: Operation | null, error?: unknown): void {
  if (!op) return;
  if (error === undefined) activity.end(op, 'ok');
  else activity.end(op, isCancellation(error) ? 'cancelled' : 'error', error);
}

/** Marks a proxy made by `withFailureLogging`, so the coverage guard can check every backend is behind one. */
export const ACTIVITY_TRACKED = Symbol.for('imt.activity.tracked');

/** True when `backend` is behind the shared proxy (every call is attributed). */
export function isActivityTracked(backend: unknown): boolean {
  return !!backend && (backend as Record<symbol, unknown>)[ACTIVITY_TRACKED] === true;
}
