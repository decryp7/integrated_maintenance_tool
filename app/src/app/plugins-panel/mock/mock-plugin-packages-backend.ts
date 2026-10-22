import { CmdError } from '../../core/cmd-error';
import { InstalledPluginDto, InstalledPluginsDto, PackageReviewDto, PermissionDto, PermissionRequestDto } from '../../core/api.types';
import { PluginPackagesBackend } from '../../core/tools-backend';

/** What the mock pretends about the app (`?mock-plugin-dev=1`, `?mock-safe=1`, `?mock-plugin-damaged=1`, `?mock-plugin-package=unsigned`). */
export interface MockPluginPackagesOptions {
  /** Developer mode: unsigned packages install with a warning instead of being refused. */
  developerMode: boolean;
  /** Safe mode: third-party plugins are not loaded. */
  safeMode: boolean;
  /** One installed record cannot be read. */
  damaged: boolean;
  /** Which package every path is (`tampered`, `unsigned`, `unknown-key`, `update`; empty: decided by the path). */
  variant: string;
}

/** Reads the `mock-*` query parameters the packages mock understands. */
export function packagesOptionsFromSearch(search: string): MockPluginPackagesOptions {
  const q = new URLSearchParams(search);
  return { developerMode: q.get('mock-plugin-dev') === '1', safeMode: q.get('mock-safe') === '1', damaged: q.get('mock-plugin-damaged') === '1', variant: q.get('mock-plugin-package') ?? '' };
}

const ALLOWS: Record<PermissionDto, string> = {
  openedFilePaths: 'Read the files you open in Debrief.',
  runPrograms: 'Start programs on this computer.',
  writeFolders: 'Write files into folders you choose.',
  network: 'Connect to servers on the network or the internet.',
  readFolders: 'Read files in folders you configure.',
};

function request(permission: PermissionDto, reason: string): PermissionRequestDto {
  return { permission, reason, allows: ALLOWS[permission] };
}

/** The plugin installed from the start. */
function sapPlugin(): InstalledPluginDto {
  return {
    id: 'com.contoso.sap-logs',
    name: 'SAP log importer',
    version: '1.2.0',
    publisher: 'Contoso',
    description: 'Reads SAP application server traces.',
    signature: { kind: 'trusted', signer: 'Contoso Ltd' },
    warning: null,
    requested: [request('network', 'To look up SAP message texts.')],
    granted: ['network'],
    enabled: true,
    state: 'running',
    problem: null,
    installedAtMs: Date.UTC(2026, 8, 1),
  };
}

/** A 64-character hex digest derived from the path (stable per path). */
function digestOf(path: string): string {
  let h = 2166136261;
  for (const c of path) h = Math.imul(h ^ c.charCodeAt(0), 16777619) >>> 0;
  return h.toString(16).padStart(8, '0').repeat(8);
}

/**
 * In-memory stand-in for the `plugin_package_*` commands, for browser development and UI tests. The path decides the
 * package: `tampered` fails the signature check, `unsigned` / `unknown-key` are not signed by a trusted publisher
 * (refused unless developer mode), `update` is a new version of the installed SAP plugin that asks for one more
 * permission; anything else is a trusted "Web access log enricher".
 */
export class MockPluginPackagesBackend extends PluginPackagesBackend {
  private plugins: InstalledPluginDto[] = [sapPlugin()];
  /** The records that cannot be read. */
  private damaged: InstalledPluginsDto['damaged'];
  /** Everything the UI asked to be done, for assertions. */
  readonly calls: string[] = [];

  constructor(
    readonly options: MockPluginPackagesOptions = packagesOptionsFromSearch(typeof location === 'undefined' ? '' : location.search),
  ) {
    super();
    this.damaged = options.damaged
      ? [{ id: 'org.example.broken', message: 'The plugin record is unreadable (manifest.toml: missing field `version`). Remove it and install the package again.' }]
      : [];
  }


  async listPackages(): Promise<InstalledPluginsDto> {
    return { plugins: this.plugins.map((p) => ({ ...p })), damaged: [...this.damaged], developerMode: this.options.developerMode, safeMode: this.options.safeMode };
  }

  async readPackage(path: string): Promise<PackageReviewDto> {
    this.calls.push(`inspect:${path}`);
    if (!path.toLowerCase().endsWith('.debriefplugin')) throw new CmdError({ code: 'invalid', message: 'This is not a Debrief plugin package (.debriefplugin).', retryable: false });
    const kind = `${path} ${this.options.variant}`;
    if (kind.includes('tampered')) {
      throw new CmdError({
        code: 'signatureInvalid',
        message: 'The package was changed after it was signed.',
        retryable: false,
        hint: 'Get the package again from its publisher.',
        detail: 'tampered',
      });
    }
    const dev = this.options.developerMode;
    if (kind.includes('update')) {
      const current = this.plugins.find((p) => p.id === 'com.contoso.sap-logs');
      return {
        ...this.base(path),
        id: 'com.contoso.sap-logs',
        name: 'SAP log importer',
        version: '1.3.0',
        publisher: 'Contoso',
        description: 'Reads SAP application server traces, now with gateway logs.',
        signature: { kind: 'trusted', signer: 'Contoso Ltd' },
        permissions: [
          request('network', 'To look up SAP message texts.'),
          request('runPrograms', 'To run the SAP trace converter.'),
        ],
        update: current ? { fromVersion: current.version, added: ['runPrograms'], removed: [] } : null,
      };
    }
    if (kind.includes('unsigned') || kind.includes('unknown-key')) {
      const unsigned = kind.includes('unsigned');
      return {
        ...this.base(path),
        id: 'org.example.homebrew',
        name: 'Homebrew parser',
        version: '0.1.0',
        publisher: 'Example',
        description: 'A parser someone built for their own logs.',
        signature: unsigned ? { kind: 'unsigned', signer: null } : { kind: 'unknownKey', signer: '7f3a9c21' },
        permissions: [request('network', 'To fetch its format definitions.')],
        refusal: dev
          ? null
          : unsigned
            ? { reason: 'unsigned', message: 'The package is not signed, so Debrief cannot tell who made it.' }
            : { reason: 'unknownKey', message: 'The package is signed with a key Debrief does not trust.' },
        warning: dev ? 'Developer mode: this package is not verified. Install it only if you built it yourself.' : null,
      };
    }
    return this.base(path);
  }

  async installPackage(path: string, digest: string, approved: PermissionDto[]): Promise<InstalledPluginDto> {
    this.calls.push(`install:${path}:${digest}:${approved.join(',')}`);
    const review = await this.readPackage(path);
    if (review.refusal) throw new CmdError({ code: 'invalid', message: review.refusal.message, retryable: false, detail: review.refusal.reason });
    if (digest !== review.digest) throw new CmdError({ code: 'invalid', message: 'The package changed since you reviewed it.', retryable: false, hint: 'Review it again.' });
    const asked = new Set(review.permissions.map((p) => p.permission));
    const installed: InstalledPluginDto = {
      id: review.id,
      name: review.name,
      version: review.version,
      publisher: review.publisher,
      description: review.description,
      signature: review.signature,
      warning: review.warning,
      requested: review.permissions,
      granted: approved.filter((p) => asked.has(p)),
      enabled: true,
      state: 'pendingRestart',
      problem: null,
      installedAtMs: Date.now(),
    };
    this.plugins = [...this.plugins.filter((p) => p.id !== review.id), installed];
    return { ...installed };
  }

  async removePackage(id: string): Promise<boolean> {
    this.calls.push(`remove:${id}`);
    const before = this.plugins.length + this.damaged.length;
    this.plugins = this.plugins.filter((p) => p.id !== id);
    this.damaged = this.damaged.filter((d) => d.id !== id);
    return this.plugins.length + this.damaged.length < before;
  }

  async setPackageEnabled(id: string, enabled: boolean): Promise<InstalledPluginDto> {
    this.calls.push(`setEnabled:${id}:${enabled}`);
    return this.update(id, (p) => ({ ...p, enabled, state: enabled ? 'pendingRestart' : 'disabled' }));
  }

  async setPackagePermissions(id: string, granted: PermissionDto[]): Promise<InstalledPluginDto> {
    this.calls.push(`setPermissions:${id}:${granted.join(',')}`);
    return this.update(id, (p) => {
      const asked = new Set(p.requested.map((r) => r.permission));
      const unknown = granted.find((g) => !asked.has(g));
      if (unknown) throw new CmdError({ code: 'invalid', message: `The plugin does not ask for ${unknown}.`, retryable: false, field: 'granted' });
      return { ...p, granted: [...granted] };
    });
  }

  /** Test hook: puts a plugin into a state (e.g. quarantined). */
  setState(id: string, state: InstalledPluginDto['state'], problem: string | null = null): void {
    this.plugins = this.plugins.map((p) => (p.id === id ? { ...p, state, problem } : p));
  }

  private update(id: string, change: (p: InstalledPluginDto) => InstalledPluginDto): InstalledPluginDto {
    const current = this.plugins.find((p) => p.id === id);
    if (!current) throw new CmdError({ code: 'notFound', message: `No plugin ${id} is installed.`, retryable: false });
    const next = change(current);
    this.plugins = this.plugins.map((p) => (p.id === id ? next : p));
    return { ...next };
  }

  private base(path: string): PackageReviewDto {
    return {
      id: 'net.example.web-enricher',
      name: 'Web access log enricher',
      version: '2.0.1',
      publisher: 'Example Software',
      description: 'Adds GeoIP and user-agent fields to web server access logs.',
      protocol: '1.0',
      targets: ['x86_64-pc-windows-msvc', 'x86_64-unknown-linux-gnu'],
      digest: digestOf(path),
      signature: { kind: 'trusted', signer: 'Example Software Inc.' },
      permissions: [request('runPrograms', 'To run the GeoIP database updater.'), request('network', 'To download the GeoIP database.')],
      refusal: null,
      warning: null,
      developerMode: this.options.developerMode,
      update: null,
    };
  }
}
