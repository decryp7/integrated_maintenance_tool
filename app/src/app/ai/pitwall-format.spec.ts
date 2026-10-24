import { describe, expect, it } from 'vitest';
import { AiEngineStatus, PitwallStatus } from './ai.types';
import { engineLines, engineSentence, groupCode } from './pitwall-format';

const pw = (o: Partial<PitwallStatus> = {}): PitwallStatus => ({
  found: 'absent', version: null, paired: false, pairing: null, model: null, usable: false, problem: null, ...o,
});
const eng = (active: AiEngineStatus['active'], p: Partial<PitwallStatus> = {}): AiEngineStatus => ({
  choice: 'auto', active, builtInForSession: false, pitwall: pw(p),
});

describe('pitwall-format', () => {
  it('groups a six-digit code and leaves anything else alone', () => {
    expect(groupCode('482913')).toBe('482 913');
    expect(groupCode('1234')).toBe('1234');
    expect(groupCode('')).toBe('');
  });

  it('names Pitwall with version and model when it is the engine', () => {
    expect(engineSentence(eng('pitwall', { version: '1.0.4', model: 'qwen3-8b' }))).toBe('Engine: Pitwall 1.0.4 · model qwen3-8b');
    expect(engineSentence(eng('pitwall'))).toBe('Engine: Pitwall');
    expect(engineSentence(eng('pitwall', { version: '1.0.4' }))).toBe('Engine: Pitwall 1.0.4');
  });

  it('says built-in alone, or with what to do about Pitwall', () => {
    expect(engineSentence(eng('builtIn'))).toBe('Engine: built-in');
    expect(engineSentence(eng('builtIn', { found: 'running' }))).toBe(
      'Engine: built-in. Pitwall is running on this computer. Connect Debrief to it to share its models.',
    );
    expect(engineSentence(eng('builtIn', { found: 'running', paired: true }))).toBe('Engine: built-in');
    expect(engineSentence(eng('builtIn', { found: 'installed' }))).toContain('Pitwall is installed but not running. Open Pitwall to use it.');
    expect(engineSentence(eng('builtIn', { found: 'incompatible', version: '3.0.0' }))).toContain(
      'Pitwall 3.0.0 speaks a newer connection version; update Debrief.',
    );
  });

  it('says no engine when Pitwall was chosen and cannot be used, with its problem', () => {
    const e = eng(null, { found: 'installed', problem: 'Pitwall stopped.' });
    expect(engineSentence(e)).toContain('Engine: none');
    expect(engineLines(e)).toEqual([engineSentence(e), 'Pitwall is installed but not running. Open Pitwall to use it.', 'Pitwall stopped.']);
    expect(engineLines(eng('pitwall', { problem: 'x' }))).toEqual(['Engine: Pitwall', 'x']);
    expect(engineLines(eng('builtIn'))).toEqual(['Engine: built-in']);
  });
});
