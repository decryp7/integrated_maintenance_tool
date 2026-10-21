import { describe, expect, it } from 'vitest';
import { fitLabel, formatBytes, formatEta, formatRate, gpuLabel, hardwareLines, percent, speedLabel } from './ai-format';

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
});
