import { DebriefCommand } from '../extensions/tokens';
import { matchWord, plainEntry, rankCommands, showsCategory, titleParts } from './palette-match';

const cmd = (id: string, title: string, category?: string): DebriefCommand => ({ id, title, ...(category ? { category } : {}), run: () => undefined });

describe('palette matching (F1)', () => {
  it('matches word starts, longer substrings and initials, not scattered letters', () => {
    expect(matchWord('anom', 'Show anomalies')?.indices).toEqual([5, 6, 7, 8]);
    expect(matchWord('anom', 'Full screen does: hide tool windows')).toBeNull();
    expect(matchWord('anom', 'Previous time range')).toBeNull();
    expect(matchWord('mal', 'Show anomalies')?.indices).toEqual([8, 9, 10]);
    expect(matchWord('ma', 'Show anomalies')).toBeNull();
    expect(matchWord('sa', 'Show anomalies')?.indices).toEqual([0, 5]);
    expect(matchWord('s', 'Show anomalies')?.indices).toEqual([0]);
    expect(matchWord('', 'x')).toEqual({ score: 0, indices: [] });
    // A word start beats the same letters inside a word.
    expect(matchWord('time', 'Previous time range')!.score).toBeGreaterThan(matchWord('time', 'Lifetime')!.score);
  });

  it('shows the category once: not beside a title that starts with it ("AI · AI: …")', () => {
    expect(showsCategory({ title: 'AI: Open the assistant chat', category: 'AI' })).toBe(false);
    expect(showsCategory({ title: 'Profiling: hotspots', category: 'profiling' })).toBe(false);
    expect(showsCategory({ title: 'View: Jobs', category: 'Window' })).toBe(true);
    expect(showsCategory({ title: 'Open folder' })).toBe(false);
    expect(plainEntry(cmd('a', 'Open folder', 'File'))).toMatchObject({ title: 'Open folder', showCategory: true });
  });

  it('ranks by every query word, better matches first, and rejects loose matches', () => {
    const commands = [
      cmd('a', 'Full screen does: hide tool windows', 'View'),
      cmd('b', 'Previous time range', 'Analyze'),
      cmd('c', 'Show anomalies', 'Analyze'),
      cmd('d', 'Open Anomalies', 'View'),
      cmd('e', 'Theme: console', 'View'),
    ];
    expect(rankCommands(commands, 'anom').map((e) => e.command.id)).toEqual(['c', 'd']);
    expect(rankCommands(commands, 'open anom').map((e) => e.command.id)).toEqual(['d']);
    // A query typed like the full label ("Category: title") still finds the command.
    expect(rankCommands(commands, 'view: theme').map((e) => e.command.id)).toEqual(['e']);
    expect(rankCommands(commands, 'zzz')).toEqual([]);
    const marks = rankCommands(commands, 'anom')[1]!.marks;
    expect(titleParts('Open Anomalies', marks)).toEqual([
      { text: 'Open ', mark: false },
      { text: 'Anom', mark: true },
      { text: 'alies', mark: false },
    ]);
  });

  it('with no query lists the recent commands, then the common ones, then the rest in order', () => {
    const commands = [cmd('x', 'Capture diagnostic screenshot'), cmd('y', 'Open folder…'), cmd('z', 'Search events'), cmd('w', 'Report a problem…')];
    expect(rankCommands(commands, '  ', ['z'], ['y', 'missing']).map((e) => e.command.id)).toEqual(['z', 'y', 'x', 'w']);
    expect(rankCommands(commands, '').map((e) => e.command.id)).toEqual(['x', 'y', 'z', 'w']);
  });
});
