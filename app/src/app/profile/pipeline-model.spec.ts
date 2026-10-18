import { pluginInfo } from '../../testing/helpers';
import { PipelineStep } from '../core/profile.model';
import {
  defaultStep,
  describeCondition,
  describeWhen,
  duplicateStep,
  formatStats,
  kindInfo,
  moveItem,
  SCRIPT_HELP,
  SCRIPT_TEMPLATE,
  scriptLines,
  STEP_CATALOG,
  stepProblem,
  stepStatus,
  summarizeStep,
  transformPlugins,
} from './pipeline-model';
import { PIPELINE_STEP_KINDS } from '../core/profile.model';
import { at } from '../../testing/must';

describe('pipeline model', () => {
  it('catalogues every step kind exactly once', () => {
    expect(STEP_CATALOG.map((k) => k.kind).sort()).toEqual([...PIPELINE_STEP_KINDS].sort());
    expect(kindInfo('drop')?.group).toBe('Filter');
    expect(kindInfo('nope')).toBeUndefined();
  });

  it('creates a default for every kind that summarises without error', () => {
    for (const { kind } of STEP_CATALOG) {
      const step = defaultStep(kind, 'p1');
      expect(step.step).toBe(kind);
      expect(summarizeStep(step)).not.toContain('Unknown');
    }
    expect(defaultStep('plugin', 'p1').id).toBe('p1');
    expect(defaultStep('plugin').id).toBe('');
  });

  it('describes conditions in English', () => {
    expect(describeCondition({ field: 'SourceContext', op: 'contains', value: 'HealthChecks' })).toBe('SourceContext contains HealthChecks');
    expect(describeCondition({ field: 'X', op: 'exists' })).toBe('X exists');
    expect(describeCondition({ field: '', op: 'eq' })).toBe('(field) equals');
    expect(describeWhen(undefined)).toBe('');
    expect(describeWhen([{ field: 'a', op: 'gt', value: '1' }, { field: 'b', op: 'missing' }])).toBe(' where a > 1 and b is missing');
  });

  it('summarises each step kind', () => {
    const s = (step: PipelineStep) => summarizeStep(step);
    expect(s({ step: 'drop', when: [{ field: 'SourceContext', op: 'contains', value: 'HealthChecks' }] })).toBe('Drop events where SourceContext contains HealthChecks');
    expect(s({ step: 'drop' })).toBe('Drop every event');
    expect(s({ step: 'keep' })).toBe('Keep only every event');
    expect(s({ step: 'keep', when: [{ field: '@level', op: 'eq', value: 'Error' }] })).toBe('Keep only events where @level equals Error');
    expect(s({ step: 'dedupe', fields: ['@message', 'X'] })).toBe('Collapse repeated events with the same @message, X within 5s');
    expect(s({ step: 'dedupe', window: '1m' })).toContain('(none) within 1m');
    expect(s({ step: 'mask', regex: 'a', replace: 'b' })).toBe('Mask text matching /a/ in @message with "b"');
    expect(s({ step: 'mask', field: 'F' })).toBe('Mask text matching // in F with ""');
    expect(s({ step: 'remove_fields', fields: ['a', ''] })).toBe('Remove fields a');
    expect(s({ step: 'truncate', field: 'F', max: 10 })).toBe('Truncate F to 10 characters');
    expect(s({ step: 'truncate' })).toBe('Truncate @message to ? characters');
    expect(s({ step: 'rename', from: 'a', to: 'b' })).toBe('Rename field a to b');
    expect(s({ step: 'rename' })).toBe('Rename field (from) to (to)');
    expect(s({ step: 'set', field: 'X', value: 'v' })).toBe('Set X to "v"');
    expect(s({ step: 'set' })).toBe('Set (field) to ""');
    expect(s({ step: 'set_level', level: 'Warning' })).toBe('Set level to Warning');
    expect(s({ step: 'set_level' })).toBe('Set level to ?');
    expect(s({ step: 'lookup', field: 'K', table: { a: {}, b: {} } })).toBe('Look up K in a table of 2 keys');
    expect(s({ step: 'lookup', file: 'x.csv' })).toBe('Look up (field) in file x.csv');
    expect(s({ step: 'parse_json', prefix: 'j.' })).toBe('Parse JSON in @message into fields prefixed "j."');
    expect(s({ step: 'parse_kv', field: 'F' })).toBe('Parse key=value pairs in F into fields');
    expect(s({ step: 'shift_time', by: '+2s' })).toBe('Shift timestamps by +2s');
    expect(s({ step: 'shift_time' })).toBe('Shift timestamps by ?');
    expect(s({ step: 'calc', field: 'ElapsedSec', expr: 'ElapsedMs / 1000' })).toBe('Set ElapsedSec = ElapsedMs / 1000');
    expect(s({ step: 'calc' })).toBe('Set (field) = (expression)');
    expect(s({ step: 'script', script: '\nif true { drop(); }\r\nlevel = "Error";\n\n' })).toBe('Run script (2 lines)');
    expect(s({ step: 'script', script: 'drop();', when: [{ field: 'X', op: 'exists' }] })).toBe('Run script (1 line) where X exists');
    expect(s({ step: 'script' })).toBe('Run script (0 lines)');
    expect(s({ step: 'plugin', id: 'geo' })).toBe('Run transform plugin "geo"');
    expect(s({ step: 'plugin' })).toContain('(choose one)');
    expect(s({ step: 'wat' as never })).toBe('Unknown step "wat"');
  });

  it('moves and duplicates steps without mutating', () => {
    const a = [1, 2, 3, 4];
    expect(moveItem(a, 0, 2)).toEqual([2, 3, 1, 4]);
    expect(moveItem(a, 3, 0)).toEqual([4, 1, 2, 3]);
    expect(moveItem(a, 1, 99)).toEqual([1, 3, 4, 2]);
    expect(moveItem(a, 1, -5)).toEqual([2, 1, 3, 4]);
    expect(moveItem(a, 9, 0)).toEqual(a);
    expect(a).toEqual([1, 2, 3, 4]);
    const steps: PipelineStep[] = [{ step: 'drop', when: [{ field: 'a', op: 'eq', value: '1' }] }, { step: 'keep' }];
    const dup = duplicateStep(steps, 0);
    expect(dup.map((x) => x.step)).toEqual(['drop', 'drop', 'keep']);
    expect(dup[1]).toEqual(dup[0]);
    expect(dup[1]).not.toBe(dup[0]);
    expect(duplicateStep(steps, 5)).toEqual(steps);
  });

  it('finds transform plugins', () => {
    const plugins = [
      pluginInfo({ id: 'a', name: 'A', contributes: ['importer'] }),
      pluginInfo({ id: 'b', name: 'B', contributes: ['exporter', 'transform:geoip'] }),
      pluginInfo({ id: 'c', name: 'C', contributes: ['transform'] }),
    ];
    expect(transformPlugins(plugins).map((p) => p.id)).toEqual(['b', 'c']);
  });

  it('formats and decides step stats', () => {
    expect(formatStats(31204, 18950)).toBe('31,204 in → 18,950 out (−12,254)');
    expect(formatStats(5, 5)).toBe('5 in → 5 out (±0)');
    expect(formatStats(1, 3)).toBe('1 in → 3 out (+2)');
    const stats = [{ index: 1, step: 'drop', input: 10, output: 7 }];
    const drop: PipelineStep = { step: 'drop' };
    expect(stepStatus(drop, 1, stats, false)).toEqual({ state: 'ok', text: '10 in → 7 out (−3)' });
    expect(stepStatus(drop, 1, stats, true).text).toBe('not run yet');
    expect(stepStatus(drop, 0, stats, false).text).toBe('not run yet');
    expect(stepStatus({ step: 'keep' }, 1, stats, false).state).toBe('stale');
    expect(stepStatus({ step: 'drop', disabled: true }, 1, stats, false)).toEqual({ state: 'disabled', text: 'disabled' });
    const failed = [{ index: 0, step: 'script', input: 1200, output: 1100, errors: 1500 }];
    expect(stepStatus({ step: 'script' }, 0, failed, false)).toEqual({ state: 'failed', text: '1,200 in → 1,100 out (−100), 1,500 failed' });
    expect(stepStatus({ step: 'script' }, 0, [{ ...at(failed, 0), errors: 0 }], false).state).toBe('ok');
  });

  it('starts a script step from a template and explains the language', () => {
    expect(defaultStep('script')).toEqual({ step: 'script', script: SCRIPT_TEMPLATE });
    expect(scriptLines(SCRIPT_TEMPLATE)).toBe(2);
    expect(scriptLines(undefined)).toBe(0);
    for (const word of ['message', 'level', 'timestamp', 'fields', 'source', 'drop()', 'regex_replace']) expect(SCRIPT_HELP).toContain(word);
    expect(kindInfo('script')).toMatchObject({ group: 'Extend', label: 'Script' });
  });

  it('finds the engine error that belongs to a step', () => {
    const error = 'pipeline step 2 (script): `script` syntax error at line 3, column 7: Expecting \';\'';
    expect(stepProblem(error, 1)).toBe("`script` syntax error at line 3, column 7: Expecting ';'");
    expect(stepProblem(`invalid profile: ${error}`, 1)).toContain('line 3, column 7');
    expect(stepProblem(error, 0)).toBeNull();
    expect(stepProblem('pipeline step 1 (calc): `expr` error at column 3: x\n  a +\n    ^', 0)).toBe('`expr` error at column 3: x\n  a +\n    ^');
    expect(stepProblem('[[marker]] is missing a kind', 0)).toBeNull();
    expect(stepProblem(null, 0)).toBeNull();
    expect(stepProblem(undefined, 0)).toBeNull();
  });
});
