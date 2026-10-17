/** Pure display formatters for the AI views (sizes, rates, times, fit and speed labels, hardware). */
import { Fit, Gpu, Hardware, Speed } from './ai.types';

const KB = 1024;
const MB = KB * 1024;
const GB = MB * 1024;

/** "5.6 GB", "812 MB", "12 KB", "0 B". */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
  if (bytes >= GB) return `${(bytes / GB).toFixed(1)} GB`;
  if (bytes >= MB) return `${Math.round(bytes / MB)} MB`;
  if (bytes >= KB) return `${Math.round(bytes / KB)} KB`;
  return `${Math.round(bytes)} B`;
}

/** "48.0 MB/s"; empty while unknown. */
export function formatRate(bytesPerSec: number): string {
  if (!Number.isFinite(bytesPerSec) || bytesPerSec <= 0) return '';
  return `${(bytesPerSec / MB).toFixed(1)} MB/s`;
}

/** "about 2 min", "about 40 s", "less than a second"; empty while unknown. */
export function formatEta(secs: number | null | undefined): string {
  if (secs == null || !Number.isFinite(secs) || secs < 0) return '';
  if (secs < 1) return 'less than a second';
  if (secs < 60) return `about ${Math.ceil(secs)} s`;
  if (secs < 3600) return `about ${Math.round(secs / 60)} min`;
  const h = Math.floor(secs / 3600);
  const m = Math.round((secs % 3600) / 60);
  return m ? `about ${h} h ${m} min` : `about ${h} h`;
}

/** 0..100 (integer) for a progress bar. */
export function percent(done: number, total: number): number {
  if (!(total > 0) || !(done > 0)) return 0;
  return Math.min(100, Math.round((done / total) * 100));
}

export function fitLabel(fit: Fit): string {
  return { comfortable: 'Fits comfortably', tight: 'Tight fit', tooLarge: 'Too large' }[fit];
}

export function speedLabel(speed: Speed): string {
  return { fast: 'Fast', moderate: 'Moderate speed', slow: 'Slow' }[speed];
}

export function gpuLabel(gpu: Gpu): string {
  if (gpu.unified) return `${gpu.name} (shared memory)`;
  return gpu.vramGb != null ? `${gpu.name} · ${Math.round(gpu.vramGb)} GB video memory` : gpu.name;
}

/** One line per part: GPU(s), RAM, CPU. */
export function hardwareLines(hw: Hardware): string[] {
  const gpus = hw.gpus.length ? hw.gpus.map(gpuLabel) : ['No GPU detected (the model runs on the CPU)'];
  return [...gpus, `${Math.round(hw.totalRamGb)} GB RAM (${Math.round(hw.availableRamGb)} GB free)`, `${hw.cpuCores} CPU cores`];
}
