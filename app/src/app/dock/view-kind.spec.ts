import type { ImtView } from '../extensions/tokens';
import { TOOL_VIEWS, viewKind } from './view-kind';

const view = (id: string, defaultPosition?: string) => ({ id, defaultPosition }) as Pick<ImtView, 'id' | 'defaultPosition'>;

describe('viewKind', () => {
  it('treats the known tool views as tools whatever their position', () => {
    for (const id of TOOL_VIEWS) expect(viewKind(view(id, 'center'))).toBe('tool');
    expect(viewKind(view('inspector', 'bottom'))).toBe('tool');
  });

  it('treats the known document views as documents even on the right or bottom', () => {
    for (const id of ['events', 'query', 'query-results', 'report', 'anomalies', 'patterns', 'exceptions', 'profiling-threads']) {
      expect(viewKind(view(id, 'right'))).toBe('document');
      expect(viewKind(view(id, 'bottom'))).toBe('document');
    }
  });

  it('falls back to the default position for unknown views', () => {
    expect(viewKind(view('plugin-x', 'right'))).toBe('tool');
    expect(viewKind(view('plugin-x', 'bottom'))).toBe('document');
    expect(viewKind(view('plugin-x', 'center'))).toBe('document');
    expect(viewKind(view('plugin-x'))).toBe('document');
  });

  it('is a document for a missing view', () => {
    expect(viewKind(undefined)).toBe('document');
    expect(viewKind(null)).toBe('document');
  });
});
