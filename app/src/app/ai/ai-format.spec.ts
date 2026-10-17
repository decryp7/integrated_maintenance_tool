import { describe, expect, it } from 'vitest';
import { fitLabel, formatBytes, formatEta, formatRate, gpuLabel, hardwareLines, percent, speedLabel } from './ai-format';

describe('ai-format', () => {
  it('formats sizes', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(Number.NaN)).toBe('0 B');
    expect(formatBytes(512)).toBe('512 B');
    expect(formatBytes(12 * 1024)).toBe('12 KB');
    expect(formatBytes(812 * 1024 ** 2)).toBe('812 MB');
    expect(formatBytes(5.6 * 1024 ** 3)).toBe('5.6 GB');
  });

  it('formats rates and times', () => {
    expect(formatRate(0)).toBe('');
    expect(formatRate(48 * 1024 ** 2)).toBe('48.0 MB/s');
    expect(formatEta(null)).toBe('');
    expect(formatEta(-1)).toBe('');
    expect(formatEta(0.2)).toBe('less than a second');
    expect(formatEta(40)).toBe('about 40 s');
    expect(formatEta(130)).toBe('about 2 min');
    expect(formatEta(3600)).toBe('about 1 h');
    expect(formatEta(5400)).toBe('about 1 h 30 min');
    expect(percent(1, 0)).toBe(0);
    expect(percent(0, 10)).toBe(0);
    expect(percent(5, 10)).toBe(50);
    expect(percent(20, 10)).toBe(100);
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
});
