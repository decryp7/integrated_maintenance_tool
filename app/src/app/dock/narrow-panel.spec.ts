import { describe, expect, it } from 'vitest';
import { NARROW_PANEL_PX, narrowFlag } from './angular-panel';

describe('narrowFlag', () => {
  it('flags a measured width below the threshold only', () => {
    expect(narrowFlag(NARROW_PANEL_PX - 1)).toBe('');
    expect(narrowFlag(1)).toBe('');
    expect(narrowFlag(NARROW_PANEL_PX)).toBeNull();
    expect(narrowFlag(1200)).toBeNull();
  });

  it('does not flag a panel that has not been measured yet (width 0)', () => {
    expect(narrowFlag(0)).toBeNull();
  });
});
