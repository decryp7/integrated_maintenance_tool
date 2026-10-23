/**
 * The inference runtimes of the mock AI backend: the llama-server builds a Windows laptop with an NVIDIA
 * GPU would be offered (sizes of the pinned release), which are installed and work, and scripted
 * downloads. Mimics `debrief_ai::runtimes::overview` closely enough for the UI; the engine is the reference.
 */
import { AiError } from '../../ai/ai-backend';
import { MockClock, systemMockClock } from './mock-clock';
import {
  AiErrorCode,
  AiInstalledRuntime,
  AiRuntimeDevice,
  AiRuntimeEntry,
  AiRuntimeEvent,
  AiRuntimeOverview,
  ManualFile,
} from '../../ai/ai.types';

/** `installed`: Vulkan installed and working (CUDA suggested); `none`: nothing installed; `broken`: Vulkan installed but it sees no GPU. */
export type MockRuntimeMode = 'installed' | 'none' | 'broken';

/** How a download fails on a corporate network (`?mockAiNet=`), as the engine classifies it. */
export const MOCK_NET_FAILURES = ['tls', 'proxyAuth', 'proxy', 'offline', 'timeout', 'httpStatus'] as const;
export type MockNetFailure = (typeof MOCK_NET_FAILURES)[number];

/** The engine's message for a failed request to `host` (plausible, not the reference). */
export function mockNetError(kind: MockNetFailure, host: string): AiError {
  const messages: Record<MockNetFailure, string> = {
    tls: `the secure connection to ${host} could not be verified (invalid peer certificate: UnknownIssuer). A proxy or security tool on your network probably intercepts HTTPS (for example Zscaler): its root certificate must be installed in Windows' certificate store; ask your IT department, or download the file manually`,
    proxyAuth: 'the proxy asks for a sign-in (proxy server responded 407/407): put the user name and password in the Download proxy setting (http://user:password@proxy:port), or download the file manually',
    proxy: 'the download proxy could not be used (the connection failed): check the Download proxy setting in Settings → AI',
    offline: `cannot reach ${host} (the name could not be resolved): check that this computer is online; if your network needs a proxy, set Download proxy in Settings → AI`,
    timeout: `${host} did not answer in time (timeout: global): try again later, or download the file manually`,
    httpStatus: `${host} answered with status 403; a proxy on your network may block the download: download the file manually or ask your IT department`,
  };
  return new AiError(kind as AiErrorCode, messages[kind]);
}

/** A plausible SHA-256 for a mock asset (stable per name). */
function fakeSha(name: string): string {
  let h = 0x811c9dc5;
  let out = '';
  for (let round = 0; out.length < 64; round++) {
    for (const c of `${name}#${round}`) h = Math.imul(h ^ c.charCodeAt(0), 0x01000193) >>> 0;
    out += h.toString(16).padStart(8, '0');
  }
  return out.slice(0, 64);
}

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


function assetNames(b: (typeof BUILDS)[number]): string[] {
  return Array.from({ length: b.assets }, (_, i) =>
    i === 0 ? `llama-${MOCK_RUNTIME_TAG}-bin-win-${b.id}-x64.zip` : `cudart-llama-bin-win-${b.id}-x64.zip`,
  );
}

export class MockRuntimes {
  private installed: Installed[];
  private choice = 'auto';
  private job: { id: string; cancel: boolean } | null = null;
  private readonly handlers = new Set<(e: AiRuntimeEvent) => void>();

  constructor(
    mode: MockRuntimeMode,
    private readonly stepMs: number,
    /** Downloads fail half-way with this error (none when null); archives added by hand still install. */
    private readonly failure: AiError | null = null,
    private readonly clock: MockClock = systemMockClock,
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

  /** The archives of build `id` (as `debrief_ai::manual::runtime_files`). */
  manualFiles(id: string): ManualFile[] {
    const b = BUILDS.find((x) => x.id === id);
    if (!b) throw new AiError('notFound', `there is no inference build '${id}' for this computer`);
    const per = Math.round(b.downloadBytes / b.assets);
    return assetNames(b).map((name) => ({
      name,
      url: `https://github.com/ggml-org/llama.cpp/releases/download/${MOCK_RUNTIME_TAG}/${name}`,
      sizeBytes: per,
      sha256: fakeSha(name),
    }));
  }

  /** Installs `id` from archives downloaded by hand: every asset by name, like the engine checks them. */
  installFrom(id: string, paths: string[]): void {
    const files = this.manualFiles(id);
    const given = paths.map((p) => (p.split(/[\\/]/).pop() ?? '').toLowerCase());
    const wrong = given.find((g) => !files.some((f) => f.name.toLowerCase() === g));
    if (wrong) throw new AiError('invalid', `${wrong} is not a file of this build; it needs ${files.map((f) => f.name).join(' and ')}`);
    if (new Set(given).size !== files.length)
      throw new AiError('invalid', `this build needs ${files.map((f) => f.name).join(' and ')}: choose all of them`);
    this.start(id, true);
  }

  install(id: string): void {
    this.start(id, false);
  }

  private start(id: string, manual: boolean): void {
    const b = BUILDS.find((x) => x.id === id);
    if (!b) throw new AiError('notFound', `there is no inference build '${id}' for this computer`);
    if (this.job)
      throw new AiError(
        'busy',
        `The ${BUILDS.find((x) => x.id === this.job!.id)?.label} runtime is being downloaded; wait for it or cancel it first.`,
      );
    this.job = { id, cancel: false };
    void this.run(b, manual);
  }

  private async run(b: (typeof BUILDS)[number], manual: boolean): Promise<void> {
    const job = this.job!;
    const step = async (e: AiRuntimeEvent) => {
      if (job.cancel) throw new AiError('cancelled', 'cancelled');
      this.emit(e);
      await this.clock.sleep(this.stepMs * 2);
    };
    try {
      const per = Math.round(b.downloadBytes / b.assets);
      const names = assetNames(b);
      for (let a = 1; a <= b.assets; a++) {
        const asset = names[a - 1]!;
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
          if (this.failure && !manual && i === 3) throw this.failure;
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
      const unpackAsset = `llama-${MOCK_RUNTIME_TAG}-bin-win-${b.id}-x64.zip`;
      for (let i = 0; i <= 4; i++) {
        await step({
          stage: 'unpacking',
          id: b.id,
          asset: unpackAsset,
          done: Math.round((b.downloadBytes * i) / 4),
          total: b.downloadBytes,
        });
      }
      await step({ stage: 'finishing', id: b.id });
      await step({ stage: 'testing', id: b.id });
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
