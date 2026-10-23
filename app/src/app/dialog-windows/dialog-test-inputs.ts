/**
 * Fixture inputs for the dialogs that cannot open without any (record-derived and AI sub-dialogs, wizards that need a
 * file). Only used by tests; `dialog-all.spec.ts` fails when a dialog is added that needs inputs and has none here.
 */
const record = { id: 1, ts: 1_700_000_000_000, tsState: 'ok', level: 'Error', message: 'Failed to capture payment', template: null, exception: null, fields: { Application: 'Svc' }, source: 0, sourceName: 'app.log', line: 4, byteOffset: 0, group: '' };
const source = { id: 0, path: '/logs/app.log', name: 'app.log', group: 'app.log', importer: 'text', records: 10, dropped: 0, skipped: 0 };
const narrative = { text: 'The payment failed [#1].', citations: [{ n: 1, id: 1 }], evidence: {}, usage: {} };

export const DIALOG_TEST_INPUTS: Record<string, Record<string, unknown>> = {
  'parser-editor': { doc: { id: 'p', origin: 'user', toml: '[parser]\nid = "p"\n' } },
  'parser-import-confirm': { check: { toml: '[parser]\nid = "p"\n', id: 'p', name: 'P', version: '1', relation: 'new', existing: null } },
  'format-wizard': { path: '/logs/app.log', name: 'app.log' },
  'plugin-permissions': { pluginId: 'demo' },
  clock: { sourceId: 0 },
  mask: { record, selected: 'payment' },
  extract: { record, start: 0, end: 6 },
  'marker-override': { row: { kind: 'k', owner: 'demo', label: 'K', enabled: true, info: null, when: [], restyled: [] } },
  encoding: { source },
  'ai-narrative': { preset: narrative },
  'ai-proposal-result': { proposal: { kind: 'query', ql: 'level = Error', explanation: 'Errors only', estimatedMatches: 3, repaired: false } },
  'ai-regex': { lines: ['2024-01-01 ERROR boom'], intent: 'level' },
  'ai-propose': { kind: 'pipeline', source: { path: '/logs/app.log' }, initialGoal: 'drop noise' },
  'profile-etl': { path: '/dumps/trace.etl' },
  'open-ask': { ask: { count: 2, names: ['a.log', 'b.log'] } },
};
