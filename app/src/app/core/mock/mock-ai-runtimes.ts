/**
 * The inference runtimes of the mock AI backend: the llama-server builds a Windows laptop with an NVIDIA
 * GPU would be offered (sizes of the pinned release), which are installed and work, and scripted
 * downloads. Mimics `imt_ai::runtimes::overview` closely enough for the UI; the engine is the reference.
 */
import { AiError } from '../../ai/ai-backend';
import {
  AiInstalledRuntime,
  AiRuntimeDevice,
  AiRuntimeEntry,
  AiRuntimeEvent,
  AiRuntimeOverview,
} from '../../ai/ai.types';

/** `installed`: Vulkan installed and working (CUDA suggested); `none`: nothing installed; `broken`: Vulkan installed but it sees no GPU. */
export type MockRuntimeMode = 'installed' | 'none' | 'broken';

const MB = 1_000_000;
export const MOCK_RUNTIME_TAG = 'b11669';

const BUILDS: {
  id: string;
  label: string;
  downloadBytes: number;
  assets: number;
  unpackedBytes: number;
}[] = [
  { id: 'cpu', label: 'CPU', downloadBytes: 20 * MB, assets: 1, unpackedBytes: 43 * MB },
  {
    id: 'vulkan',
    label: 'Vulkan (any GPU vendor)',
    downloadBytes: 32 * MB,
    assets: 1,
    unpackedBytes: 71 * MB,
  },
  {
    id: 'cuda-13',
    label: 'CUDA 13 (NVIDIA)',
    downloadBytes: 578 * MB,
    assets: 2,
    unpackedBytes: 1_310 * MB,
  },
  {
    id: 'cuda-12',
    label: 'CUDA 12 (NVIDIA)',
    downloadBytes: 658 * MB,
    assets: 2,
    unpackedBytes: 1_480 * MB,
  },
  {
    id: 'rocm',
    label: 'ROCm / HIP (AMD)',
    downloadBytes: 258 * MB,
    assets: 1,
    unpackedBytes: 690 * MB,
  },
  {
    id: 'sycl',
    label: 'SYCL (Intel GPU, FP16)',
    downloadBytes: 149 * MB,
    assets: 1,
    unpackedBytes: 420 * MB,
  },
  {
    id: 'openvino',
    label: 'OpenVINO (Intel)',
    downloadBytes: 99 * MB,
    assets: 1,
    unpackedBytes: 260 * MB,
  },
];

/** The automatic order of the engine (`runtimes::builds::try_order`); ROCm, SYCL and OpenVINO only when chosen. */
const AUTO_ORDER = ['metal', 'cuda-13', 'cuda-12', 'vulkan', 'cpu'];

const GPU: Record<string, AiRuntimeDevice> = {
  vulkan: {
    name: 'Vulkan0',
    description: 'NVIDIA GeForce RTX 4060 Laptop GPU',
    totalMib: 8188,
    freeMib: 7420,
  },
  'cuda-13': {
    name: 'CUDA0',
    description: 'NVIDIA GeForce RTX 4060 Laptop GPU',
    totalMib: 8188,
    freeMib: 7536,
  },
  'cuda-12': {
    name: 'CUDA0',
    description: 'NVIDIA GeForce RTX 4060 Laptop GPU',
    totalMib: 8188,
    freeMib: 7536,
  },
};

interface Installed {
  id: string;
  tag: string;
  usable: boolean;
  problem: string | null;
}

const sleep = (ms: number) => new Promise<void>((r) => setTimeout(r, ms));

export class MockRuntimes {
  private installed: Installed[];
  private choice = 'auto';
  private job: { id: string; cancel: boolean } | null = null;
  private readonly handlers = new Set<(e: AiRuntimeEvent) => void>();

  constructor(
    mode: MockRuntimeMode,
    private readonly stepMs: number,
    /** Downloads fail with a network error half-way. */
    private readonly failDownloads: boolean,
  ) {
    this.installed =
      mode === 'none'
        ? []
        : [
            {
              id: 'vulkan',
              tag: MOCK_RUNTIME_TAG,
              usable: mode === 'installed',
              problem:
                mode === 'broken'
                  ? 'the Vulkan build did not find a supported GPU (only llvmpipe)'
                  : null,
            },
            // An older release still on disk.
            { id: 'cpu', tag: 'b10012', usable: true, problem: null },
          ];
  }

  /** The build that runs the model, as the engine would pick it. */
  active(): string | null {
    const usable = this.installed
      .filter((i) => i.tag === MOCK_RUNTIME_TAG && i.usable)
      .map((i) => i.id);
    if (this.choice !== 'auto' && usable.includes(this.choice)) return this.choice;
    return AUTO_ORDER.find((id) => usable.includes(id)) ?? null;
  }

  /** The engine's backend name for the active build. */
  backendName(): string | null {
    const a = this.active();
    return a ? (a.startsWith('cuda') ? 'CUDA' : a === 'cpu' ? 'CPU' : 'Vulkan') : null;
  }

  hasRuntime(): boolean {
    return this.active() !== null;
  }

  overview(): AiRuntimeOverview {
    const active = this.active();
    const current = this.installed.filter((i) => i.tag === MOCK_RUNTIME_TAG);
    const builds: AiRuntimeEntry[] = BUILDS.map((b) => {
      const inst = current.find((i) => i.id === b.id);
      return {
        id: b.id,
        label: b.label,
        downloadBytes: b.downloadBytes,
        installed: !!inst,
        installedBytes: inst ? b.unpackedBytes : null,
        usable: inst ? inst.usable : null,
        problem: inst?.problem ?? null,
        devices: inst?.usable && GPU[b.id] ? [structuredClone(GPU[b.id]!)] : [],
        suggested: b.id === 'cuda-13',
        active: b.id === active,
      };
    });
    const outdated: AiInstalledRuntime[] = this.installed
      .filter((i) => i.tag !== MOCK_RUNTIME_TAG)
      .map((i) => ({
        id: i.id,
        tag: i.tag,
        bytes: BUILDS.find((b) => b.id === i.id)?.unpackedBytes ?? 0,
        current: false,
      }));
    return {
      release: 'v0.7.0',
      tag: MOCK_RUNTIME_TAG,
      platform: 'windows-x64',
      choice: this.choice,
      suggested: 'cuda-13',
      active,
      customPath: null,
      builds,
      outdated,
      needsDownload: current.length === 0,
      problem: null,
    };
  }

  install(id: string): void {
    const b = BUILDS.find((x) => x.id === id);
    if (!b) throw new AiError('notFound', `there is no inference build '${id}' for this computer`);
    if (this.job)
      throw new AiError(
        'busy',
        `The ${BUILDS.find((x) => x.id === this.job!.id)?.label} runtime is being downloaded; wait for it or cancel it first.`,
      );
    this.job = { id, cancel: false };
    void this.run(b);
  }

  private async run(b: (typeof BUILDS)[number]): Promise<void> {
    const job = this.job!;
    const step = async (e: AiRuntimeEvent) => {
      if (job.cancel) throw new AiError('cancelled', 'cancelled');
      this.emit(e);
      await sleep(this.stepMs * 2);
    };
    try {
      const per = Math.round(b.downloadBytes / b.assets);
      for (let a = 1; a <= b.assets; a++) {
        const asset =
          a === 1
            ? `llama-${MOCK_RUNTIME_TAG}-bin-win-${b.id}-x64.zip`
            : `cudart-llama-bin-win-${b.id}-x64.zip`;
        await step({ stage: 'downloading', id: b.id, asset, index: a, count: b.assets });
        const steps = 6;
        for (let i = 0; i <= steps; i++) {
          const bytes = Math.round((per * i) / steps);
          await step({
            stage: 'progress',
            id: b.id,
            bytes,
            total: per,
            bytesPerSec: 24 * MB,
            etaSecs: Math.ceil((per - bytes) / (24 * MB)),
            verifying: false,
          });
          if (this.failDownloads && i === 3)
            throw new AiError('network', 'network error: the connection to github.com was reset');
        }
        await step({
          stage: 'progress',
          id: b.id,
          bytes: per,
          total: per,
          bytesPerSec: 0,
          etaSecs: null,
          verifying: true,
        });
      }
      await step({
        stage: 'unpacking',
        id: b.id,
        asset: `llama-${MOCK_RUNTIME_TAG}-bin-win-${b.id}-x64.zip`,
      });
      await step({ stage: 'finishing', id: b.id });
      // Like the worker's check: CUDA works with the laptop's NVIDIA GPU; the AMD and Intel builds find no GPU.
      const usable = !['rocm', 'sycl', 'openvino'].includes(b.id);
      this.installed = [
        ...this.installed.filter((i) => !(i.id === b.id && i.tag === MOCK_RUNTIME_TAG)),
        {
          id: b.id,
          tag: MOCK_RUNTIME_TAG,
          usable,
          problem: usable ? null : `the ${b.label} build did not find a supported GPU`,
        },
      ];
      this.job = null;
      this.emit({
        stage: 'ready',
        id: b.id,
        runtime: { id: b.id, tag: MOCK_RUNTIME_TAG, bytes: b.unpackedBytes, current: true },
      });
    } catch (e) {
      this.job = null;
      const err = e instanceof AiError ? e : new AiError('engine', String(e));
      this.emit(
        err.code === 'cancelled'
          ? { stage: 'cancelled', id: b.id }
          : { stage: 'failed', id: b.id, code: err.code, message: err.message },
      );
    }
  }

  cancel(): boolean {
    if (!this.job) return false;
    this.job.cancel = true;
    return true;
  }

  delete(tag: string, id: string): void {
    if (
      !/^[a-z0-9.-]{1,32}$/.test(tag) ||
      !/^[a-z0-9.-]{1,32}$/.test(id) ||
      tag.startsWith('.') ||
      id.startsWith('.')
    ) {
      throw new AiError('invalid', 'that is not an inference build name');
    }
    if (this.job?.id === id && tag === MOCK_RUNTIME_TAG)
      throw new AiError('busy', 'That build is being downloaded; cancel the download first.');
    if (!this.installed.some((i) => i.id === id && i.tag === tag))
      throw new AiError('notFound', `not found: the ${id} build`);
    this.installed = this.installed.filter((i) => !(i.id === id && i.tag === tag));
  }

  choose(id: string): void {
    const c = id.trim().toLowerCase();
    if (c !== 'auto' && !this.installed.some((i) => i.id === c && i.tag === MOCK_RUNTIME_TAG)) {
      throw new AiError(
        'notFound',
        `the ${BUILDS.find((b) => b.id === c)?.label ?? c} runtime is not installed; download it first`,
      );
    }
    this.choice = c;
  }

  subscribe(h: (e: AiRuntimeEvent) => void): () => void {
    this.handlers.add(h);
    return () => this.handlers.delete(h);
  }

  private emit(e: AiRuntimeEvent): void {
    for (const h of this.handlers) h(e);
  }
}
