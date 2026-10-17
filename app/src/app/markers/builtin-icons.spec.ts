import { builtinIcon, BUILTIN_ICONS, searchIcons } from './builtin-icons';

describe('icon lookup and search', () => {
  it('finds icons by name', () => {
    expect(builtinIcon('bug')?.name).toBe('bug');
    expect(builtinIcon('nope')).toBeUndefined();
    expect(builtinIcon(null)).toBeUndefined();
  });
  it('searches names and keywords with every word', () => {
    expect(searchIcons('')).toHaveLength(BUILTIN_ICONS.length);
    expect(searchIcons('sql').map((i) => i.name)).toEqual(['database']);
    expect(searchIcons('card pay').map((i) => i.name)).toEqual(['credit-card']);
    expect(searchIcons('zzz')).toEqual([]);
  });
});
