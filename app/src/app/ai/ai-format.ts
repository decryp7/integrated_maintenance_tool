/** Pure display formatters for the AI views (sizes, rates, times, fit and speed labels, hardware). */
import { Assessment, Fit, FitAdvice, Gpu, Hardware, Speed } from './ai.types';
export { formatBytes, formatConnections, formatEta, formatRate, percent } from '../shared/format/units';

/** "Text + images" or "Text only": whether a model reads images (ADR 0025). */
export function visionLabel(vision: boolean): string {
  return vision ? $localize`:@@ai.vision.models.textImages:Text + images` : $localize`:@@ai.vision.models.textOnly:Text only`;
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

const gb = (n: number): string => (Math.round(n * 10) / 10).toFixed(1);

/** The numbers behind a verdict: "Needs about 7.6 GB (model 5.3 + context 2.3) · 6.8 of 7.8 GB available now". */
export function fitNumbers(a: Assessment): string {
  const need = a.rough
    ? $localize`:@@ai.format.fit.needRough:Needs roughly ${gb(a.needGb)}:need: GB (model ${gb(a.weightsGb)}:weights: + context ${gb(a.contextGb)}:context:, estimated)`
    : $localize`:@@ai.format.fit.need:Needs about ${gb(a.needGb)}:need: GB (model ${gb(a.weightsGb)}:weights: + context ${gb(a.contextGb)}:context:)`;
  const avail = $localize`:@@ai.format.fit.available:${gb(a.availableNowGb)}:now: of ${gb(a.capacityGb)}:capacity: GB available now`;
  const others =
    a.inUseGb >= 0.5 ? $localize`:@@ai.format.fit.others: (other programs use ${gb(a.inUseGb)}:used: GB)` : '';
  return `${need} · ${avail}${others}`;
}

/** "Fits this Mac" / "Fits right now" lines: the two facts behind the verdict. */
export function fitFacts(a: Assessment): { machine: string; now: string } {
  const word = (f: Fit) => ({ comfortable: $localize`:@@ai.format.fit.yes:yes`, tight: $localize`:@@ai.format.fit.barely:barely`, tooLarge: $localize`:@@ai.format.fit.no:no` })[f];
  return {
    machine: $localize`:@@ai.format.fit.machine:Fits this computer: ${word(a.fitsMachine)}:answer:`,
    now: $localize`:@@ai.format.fit.now:Fits right now: ${word(a.fitsNow)}:answer:`,
  };
}

export function adviceLabel(x: FitAdvice): string {
  switch (x.kind) {
    case 'shorterContext':
      return $localize`:@@ai.format.advice.context:Use a shorter context (${x.tokens}:tokens: tokens)`;
    case 'smallerQuant':
      return $localize`:@@ai.format.advice.quant:Choose a smaller model or a lower quantisation`;
    case 'closeApps':
      return $localize`:@@ai.format.advice.close:Close other programs to free about ${gb(x.gb)}:gb: GB`;
  }
}

/** Advice for a model that does not fit comfortably (empty when it does). */
export function fitAdvice(a: Assessment): string[] {
  return a.advice.map(adviceLabel);
}
