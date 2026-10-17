import { Breadcrumb } from './types';

/**
 * Steps to reproduce from breadcrumbs, for the MOCK backend only (browser development and e2e). The engine's
 * `activity::steps` (app/src-tauri/src/activity/steps.rs) is the reference; this port follows its golden tests.
 */

const MAX_STEPS = 30;

const TRIGGER_PHRASE: Record<string, string> = {
  menu: 'from the menu',
  'native-menu': 'from the app menu',
  palette: 'from the command palette',
  shortcut: 'with a keyboard shortcut',
  'context-menu': 'from a context menu',
  button: 'with a button',
  'drag-drop': 'by drag and drop',
  dialog: 'in a dialog',
};

function describe(b: Breadcrumb): string {
  const p = (k: string): string | undefined => b.params?.[k] || undefined;
  const panel = p('Panel') ?? b.view;
  const thePanel = panel ? `the ${panel} panel` : 'a panel';
  const hashed = (text: string, hash: string) => (p(text) ? ` "${p(text)}"` : p(hash) ? ` (hash ${p(hash)})` : '');
  const op = b.operation;
  let s: string;
  switch (op) {
    case 'file.open': {
      const what = p('Path') ?? p('File') ?? 'files';
      s = p('Files') ? `Opened ${what} (${p('Files')} files)` : `Opened ${what}`;
      break;
    }
    case 'file.save':
      s = `Saved ${p('Path') ?? p('File') ?? 'a file'}`;
      break;
    case 'file.export':
      s = `Exported ${p('Format') ? `${p('Format')} ` : ''}${p('Path') ? `to ${p('Path')}` : 'events'}`;
      break;
    case 'query.change':
      s = `Applied query${hashed('Query', 'QueryHash')}`;
      break;
    case 'search.change':
      s = `Searched for${hashed('Search', 'SearchHash')}`;
      break;
    case 'filter.change':
      s = p('Filters') ? `Changed the filters (${p('Filters')} active)` : 'Changed the filters';
      break;
    case 'timeRange.change':
      s = 'Changed the time range';
      break;
    case 'selection.change':
      s = p('RecordId') ? `Selected event ${p('RecordId')}` : 'Changed the selection';
      break;
    case 'dock.open':
      s = `Opened ${thePanel}`;
      break;
    case 'dock.close':
      s = `Closed ${thePanel}`;
      break;
    case 'dock.popout':
      s = `Popped out ${panel ?? 'a panel'}`;
      break;
    case 'dock.dockBack':
      s = `Docked ${panel ?? 'a panel'} back`;
      break;
    case 'dock.autoHide':
      s = `Auto-hid ${panel ?? 'a panel'}`;
      break;
    case 'dock.layout':
      s = 'Applied a panel layout';
      break;
    case 'settings.change':
      s = p('Section') && p('Fields') ? `Changed settings ${p('Section')}: ${p('Fields')}` : p('Section') ? `Changed ${p('Section')} settings` : 'Changed settings';
      break;
    case 'live.on':
      s = 'Turned live tail on';
      break;
    case 'live.off':
      s = 'Turned live tail off';
      break;
    case 'ai.ask':
      s = 'Asked the AI assistant';
      break;
    case 'ai.applyProposal':
      s = 'Applied an AI proposal';
      break;
    case 'ai.webSearch.approve':
      s = 'Approved a web search';
      break;
    case 'ai.webSearch.deny':
      s = 'Denied a web search';
      break;
    case 'edit.undo':
      s = 'Undid the last change';
      break;
    case 'edit.redo':
      s = 'Redid the last change';
      break;
    default:
      s = op.startsWith('ipc.') ? `Ran ${p('Command') ?? op.slice(4)}` : p('Title') ? `Clicked '${p('Title')}'` : `Ran ${op}`;
  }
  const phrase = TRIGGER_PHRASE[b.trigger];
  return phrase && !(op.startsWith('dock.') && b.trigger === 'button') ? `${s} ${phrase}` : s;
}

function outcome(b: Breadcrumb): string {
  if (b.outcome === 'error') return ` → error ${b.errorCode ?? 'E_UNKNOWN'}`;
  if (b.outcome === 'cancelled') return ' (cancelled)';
  return '';
}

/** Numbered steps, oldest first; identical consecutive steps merged ("(×3)"); `failure` added last. */
export function steps(crumbs: readonly Breadcrumb[], failure?: string): string[] {
  const lines: [string, number][] = [];
  for (const b of crumbs) {
    const line = `${describe(b)}${outcome(b)}`;
    const last = lines[lines.length - 1];
    if (last && last[0] === line) last[1]++;
    else lines.push([line, 1]);
  }
  const skipped = Math.max(0, lines.length - MAX_STEPS);
  const out: string[] = skipped ? [`… (${skipped} earlier steps)`] : [];
  for (const [line, n] of lines.slice(skipped)) out.push(n > 1 ? `${line} (×${n})` : line);
  if (failure) out.push(failure);
  return out.map((s, i) => `${i + 1}. ${s}`);
}
