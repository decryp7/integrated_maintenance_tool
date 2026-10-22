import type { ParserEntryDto } from '../../core/api.types';
import { searchParsers } from './parser-search';

const parser = (id: string, name: string, category: string, description = ''): ParserEntryDto =>
  ({ id, name, category, description }) as unknown as ParserEntryDto;

describe('searchParsers', () => {
  const library = [parser('serilog-clef', 'Serilog CLEF', '.NET', 'Compact JSON events'), parser('nginx-access', 'nginx access', 'Web servers')];

  it('matches the section by its title and what it is about', () => {
    expect(searchParsers(library, 'Parsers', 'Parsers')).toEqual({ section: true, parsers: [] });
    expect(searchParsers(library, 'log formats', 'Parsers')?.section).toBe(true);
    expect(searchParsers(library, 'Analyseurs', 'Analyseurs')?.section).toBe(true);
  });

  it('matches parsers when every word is in their name, id, category or description', () => {
    expect(searchParsers(library, 'serilog json', 'Parsers')?.parsers.map((p) => p.id)).toEqual(['serilog-clef']);
    expect(searchParsers(library, 'web', 'Parsers')?.parsers.map((p) => p.id)).toEqual(['nginx-access']);
    expect(searchParsers(library, 'serilog web', 'Parsers')).toBeNull();
  });

  it('finds nothing for an empty query or unrelated words', () => {
    expect(searchParsers(library, '   ', 'Parsers')).toBeNull();
    expect(searchParsers(library, 'timeout', 'Parsers')).toBeNull();
    expect(searchParsers([], 'zzz', 'Parsers')).toBeNull();
  });
});
