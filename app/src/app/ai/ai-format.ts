/** Pure display formatters for the AI views (sizes, rates, times, fit and speed labels, hardware). */
import { Fit, Gpu, Hardware, Speed } from './ai.types';
export { formatBytes, formatEta, formatRate, percent } from '../shared/format/units';

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
