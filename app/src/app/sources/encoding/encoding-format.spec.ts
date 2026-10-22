import { EncodingInfo } from '../../core/api.types';
import { AUTO_ENCODING, encodingBadge, encodingText, encodingTitle, initialEncodingChoice, isLowConfidence, showsEncodingBadge } from './encoding-format';

const info = (label: string, confidence: number, origin: EncodingInfo['origin']): EncodingInfo => ({ label, confidence, origin });

describe('encoding formatting', () => {
  it('writes the encoding as one line per origin; only a guess shows its confidence', () => {
    expect(encodingText(info('Shift_JIS', 84, 'detected'))).toBe('Shift_JIS · 84% (detected)');
    expect(encodingText(info('UTF-16LE', 100, 'bom'))).toBe('UTF-16LE (byte-order mark)');
    expect(encodingText(info('GBK', 100, 'profile'))).toBe('GBK (profile)');
    expect(encodingText(info('windows-1252', 100, 'manual'))).toBe('windows-1252 (chosen)');
  });

  it('flags only a guess under 70% as doubtful, and explains it in the tooltip', () => {
    expect(isLowConfidence(info('windows-1252', 57, 'detected'))).toBe(true);
    expect(isLowConfidence(info('windows-1252', 70, 'detected'))).toBe(false);
    expect(isLowConfidence(info('windows-1252', 10, 'manual'))).toBe(false);
    expect(isLowConfidence(null)).toBe(false);
    expect(isLowConfidence(undefined)).toBe(false);
    expect(encodingTitle(info('windows-1252', 57, 'detected'))).toContain('Guessed; choose another encoding if the text looks wrong.');
    expect(encodingTitle(info('Shift_JIS', 84, 'detected'))).toBe('Shift_JIS · 84% (detected)');
  });

  it('badges the label, with the confidence of an uncertain guess', () => {
    expect(encodingBadge(info('Shift_JIS', 84, 'detected'))).toBe('Shift_JIS 84%');
    expect(encodingBadge(info('Shift_JIS', 100, 'detected'))).toBe('Shift_JIS');
    expect(encodingBadge(info('Shift_JIS', 100, 'manual'))).toBe('Shift_JIS');
  });

  it('shows a badge unless the encoding is an unremarkable UTF-8 (or there is none)', () => {
    expect(showsEncodingBadge(null)).toBe(false);
    expect(showsEncodingBadge(undefined)).toBe(false);
    expect(showsEncodingBadge(info('UTF-8', 100, 'detected'))).toBe(false);
    expect(showsEncodingBadge(info('UTF-8', 100, 'bom'))).toBe(false);
    expect(showsEncodingBadge(info('UTF-8', 50, 'detected'))).toBe(true);
    expect(showsEncodingBadge(info('UTF-8', 100, 'manual'))).toBe(true);
    expect(showsEncodingBadge(info('Shift_JIS', 100, 'detected'))).toBe(true);
  });

  it('selects the user\'s own choice at first, else Automatic', () => {
    expect(initialEncodingChoice(info('Big5', 100, 'manual'))).toBe('Big5');
    expect(initialEncodingChoice(info('Big5', 90, 'detected'))).toBe(AUTO_ENCODING);
    expect(initialEncodingChoice(info('Big5', 100, 'profile'))).toBe(AUTO_ENCODING);
    expect(initialEncodingChoice(null)).toBe(AUTO_ENCODING);
  });
});
