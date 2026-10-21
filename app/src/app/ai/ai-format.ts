/** Pure display formatters for the AI views (sizes, rates, times, fit and speed labels, hardware). */
import { Fit, Gpu, Hardware, Speed } from './ai.types';
import { localizeDecimal } from '../i18n/locale';

const KB = 1024;
const MB = KB * 1024;
const GB = MB * 1024;

/** "5.6 GB", "812 MB", "12 KB", "0 B". */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
  if (bytes >= GB) return `${localizeDecimal((bytes / GB).toFixed(1))} GB`;
  if (bytes >= MB) return `${Math.round(bytes / MB)} MB`;
  if (bytes >= KB) return `${Math.round(bytes / KB)} KB`;
  return `${Math.round(bytes)} B`;
}

/** "48.0 MB/s"; empty while unknown. */
export function formatRate(bytesPerSec: number): string {
  if (!Number.isFinite(bytesPerSec) || bytesPerSec <= 0) return '';
  return `${localizeDecimal((bytesPerSec / MB).toFixed(1))} MB/s`;
}

/** "about 2 min", "about 40 s", "less than a second"; empty while unknown. */
export function formatEta(secs: number | null | undefined): string {
  if (secs == null || !Number.isFinite(secs) || secs < 0) return '';
  if (secs < 1) return $localize`:@@ai.format.eta.underSecond:less than a second`;
  if (secs < 60) return $localize`:@@ai.format.eta.seconds:about ${Math.ceil(secs)}:seconds: s`;
  if (secs < 3600) return $localize`:@@ai.format.eta.minutes:about ${Math.round(secs / 60)}:minutes: min`;
  const h = Math.floor(secs / 3600);
  const m = Math.round((secs % 3600) / 60);
  return m
    ? $localize`:@@ai.format.eta.hoursMinutes:about ${h}:hours: h ${m}:minutes: min`
    : $localize`:@@ai.format.eta.hours:about ${h}:hours: h`;
}

/** "Text + images" or "Text only": whether a model reads images (ADR 0025). */
export function visionLabel(vision: boolean): string {
  return vision ? $localize`:@@ai.vision.models.textImages:Text + images` : $localize`:@@ai.vision.models.textOnly:Text only`;
}

/** 0..100 (integer) for a progress bar. */
export function percent(done: number, total: number): number {
  if (!(total > 0) || !(done > 0)) return 0;
  return Math.min(100, Math.round((done / total) * 100));
}

export function fitLabel(fit: Fit): string {
  return {
    comfortable: $localize`:@@ai.format.fit.comfortable:Fits comfortably`,
    tight: $localize`:@@ai.format.fit.tight:Tight fit`,
    tooLarge: $localize`:@@ai.format.fit.tooLarge:Too large`,
  }[fit];
}

export function speedLabel(speed: Speed): string {
  return {
    fast: $localize`:@@ai.format.speed.fast:Fast`,
    moderate: $localize`:@@ai.format.speed.moderate:Moderate speed`,
    slow: $localize`:@@ai.format.speed.slow:Slow`,
  }[speed];
}

export function gpuLabel(gpu: Gpu): string {
  if (gpu.unified) return $localize`:@@ai.format.gpu.shared:${gpu.name}:name: (shared memory)`;
  return gpu.vramGb != null ? $localize`:@@ai.format.gpu.vram:${gpu.name}:name: · ${Math.round(gpu.vramGb)}:gb: GB video memory` : gpu.name;
}

/** One line per part: GPU(s), RAM, CPU. */
export function hardwareLines(hw: Hardware): string[] {
  const gpus = hw.gpus.length ? hw.gpus.map(gpuLabel) : [$localize`:@@ai.format.hardware.noGpu:No GPU detected (the model runs on the CPU)`];
  return [
    ...gpus,
    $localize`:@@ai.format.hardware.ram:${Math.round(hw.totalRamGb)}:total: GB RAM (${Math.round(hw.availableRamGb)}:free: GB free)`,
    $localize`:@@ai.format.hardware.cpu:${hw.cpuCores}:cores: CPU cores`,
  ];
}
