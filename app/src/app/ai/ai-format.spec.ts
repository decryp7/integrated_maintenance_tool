import { describe, expect, it } from 'vitest';
import { Assessment } from './ai.types';
import { adviceLabel, fitAdvice, fitFacts, fitNumbers, fitLabel, formatBytes, formatEta, formatRate, gpuLabel, hardwareLines, percent, speedLabel } from './ai-format';

describe('ai-format', () => {
  it('shares the common unit formatters', () => {
    expect(formatBytes(812 * 1024 ** 2)).toBe('812 MB');
    expect(formatRate(48 * 1024 ** 2)).toBe('48 MB/s');
    expect(formatEta(130)).toBe('about 2 min left');
    expect(percent(5, 10)).toBe(50);
  });

  it('labels fit, speed and hardware', () => {
    expect(fitLabel('comfortable')).toBe('Fits comfortably');
    expect(fitLabel('tight')).toBe('Tight fit');
    expect(fitLabel('tooLarge')).toBe('Too large');
    expect(speedLabel('fast')).toBe('Fast');
    expect(speedLabel('moderate')).toBe('Moderate speed');
    expect(speedLabel('slow')).toBe('Slow');
    expect(gpuLabel({ name: 'M3', vramGb: 32, unified: true })).toBe('M3 (shared memory)');
    expect(gpuLabel({ name: 'RTX', vramGb: 8, unified: false })).toBe('RTX · 8 GB video memory');
    expect(gpuLabel({ name: 'X', vramGb: null, unified: false })).toBe('X');
    const hw = { os: 'linux', arch: 'x86_64', cpuCores: 8, totalRamGb: 16, availableRamGb: 12.4, gpus: [] };
    expect(hardwareLines(hw)).toEqual(['No GPU detected (the model runs on the CPU)', '16 GB RAM (12 GB free)', '8 CPU cores']);
    expect(hardwareLines({ ...hw, gpus: [{ name: 'RTX', vramGb: 8, unified: false }] })[0]).toBe('RTX · 8 GB video memory');
  });

  describe('the numbers behind a verdict', () => {
    const base: Assessment = {
      needGb: 7.64,
      weightsGb: 5.29,
      contextGb: 2.35,
      context: 8192,
      rough: false,
      fit: 'tight',
      speed: 'fast',
      fitsMachine: 'comfortable',
      fitsNow: 'tooLarge',
      capacityGb: 7.75,
      availableNowGb: 6.75,
      inUseGb: 1,
      spills: false,
      advice: [{ kind: 'shorterContext', tokens: 4096 }, { kind: 'smallerQuant' }, { kind: 'closeApps', gb: 0.9 }],
    };

    it('says what is needed, what is available now, and who uses the rest', () => {
      expect(fitNumbers(base)).toBe('Needs about 7.6 GB (model 5.3 + context 2.4) · 6.8 of 7.8 GB available now (other programs use 1.0 GB)');
      expect(fitNumbers({ ...base, rough: true, inUseGb: 0.1 })).toBe('Needs roughly 7.6 GB (model 5.3 + context 2.4, estimated) · 6.8 of 7.8 GB available now');
    });

    it('answers "fits this computer" and "fits right now" separately', () => {
      expect(fitFacts(base)).toEqual({ machine: 'Fits this computer: yes', now: 'Fits right now: no' });
      expect(fitFacts({ ...base, fitsMachine: 'tight', fitsNow: 'tight' })).toEqual({ machine: 'Fits this computer: barely', now: 'Fits right now: barely' });
    });

    it('suggests a shorter context, a smaller quantisation or closing programs', () => {
      expect(fitAdvice(base)).toEqual([
        'Use a shorter context (4096 tokens)',
        'Choose a smaller model or a lower quantisation',
        'Close other programs to free about 0.9 GB',
      ]);
      expect(fitAdvice({ ...base, advice: [] })).toEqual([]);
      expect(adviceLabel({ kind: 'smallerQuant' })).toContain('smaller');
    });
  });
});
