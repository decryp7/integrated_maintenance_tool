import { readdirSync, readFileSync } from 'node:fs';
import { AbstractType, EnvironmentInjector, Injector } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { MemoryLogger } from '../../../testing/helpers';
import { appConfig } from '../../app.config';
import { COMMAND_CONTEXT, CommandRegistry } from '../../extensions/command-registry';
import { CommandContext, flattenContributions, IMT_COMMANDS, ImtCommand } from '../../extensions/tokens';
import { Logger, withFailureLogging } from '../logger';
import { isActivityTracked, methodKind } from './backend-activity';
import { activity } from './runtime';

/**
 * The coverage guard: makes un-instrumented user operations impossible rather than hoping.
 *
 * - every registered command (all built-ins and plugins of the real app) produces exactly one activity entry
 *   when it runs, and nothing runs a command except the registry;
 * - every backend of the app is behind the shared proxy, so each of its methods is attributed to the current
 *   operation, and every method that changes something becomes an operation of its own when none is current;
 * - every Tauri command call goes through `core/activity/ipc.ts`, which sends the operation id to Rust (whose
 *   invoke wrapper logs every command; `app/src-tauri/src/activity/ipc.rs` has the Rust half of this guard).
 */

const SRC = 'src/app';

/** Lets the app's start-up work finish before the test module is torn down. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 50));

function sources(): { path: string; text: string }[] {
  return readdirSync(SRC, { recursive: true })
    .filter((p) => p.endsWith('.ts') && !p.endsWith('.spec.ts') && !p.includes('/testing/') && !p.includes('test-support') && !p.includes('test-setup'))
    .map((p) => ({ path: p.replaceAll('\\', '/'), text: readFileSync(`${SRC}/${p}`, 'utf8') }));
}

/** Files that still import Tauri's `invoke` directly; owned by other features (one import line each to change). */
const PENDING_DIRECT_INVOKE = new Set<string>();

describe('activity coverage guard', () => {
  beforeEach(() => activity.reset());

  it('every registered command logs exactly one entry when run', async () => {
    // The real app's providers, with the logger captured; only the command list is read from them.
    TestBed.configureTestingModule({ providers: [...appConfig.providers, { provide: Logger, useFactory: () => new MemoryLogger() }] });
    const real = flattenContributions<ImtCommand>(TestBed.inject(IMT_COMMANDS, null) ?? []);
    expect(real.length).toBeGreaterThan(50);

    // Run each through a registry of stubs (the guard is about the path, not about what commands do).
    const ran: string[] = [];
    const stubs = real.map((c) => ({ ...c, when: undefined, run: () => void ran.push(c.id) }));
    const scope = Injector.create({
      providers: [{ provide: COMMAND_CONTEXT, useValue: {} as CommandContext }, { provide: IMT_COMMANDS, multi: true, useValue: stubs }, CommandRegistry],
      parent: TestBed.inject(EnvironmentInjector),
    });
    const registry = scope.get(CommandRegistry);
    const logger = TestBed.inject(Logger) as MemoryLogger;
    const ids = [...new Set(stubs.map((c) => c.id))];
    const missing: string[] = [];
    for (const id of ids) {
      const before = logger.entries.filter((e) => e.category === 'IMT.Activity').length;
      await registry.run(id);
      const after = logger.entries.filter((e) => e.category === 'IMT.Activity');
      if (after.length !== before + 1 || at(after, after.length - 1).props?.['Operation'] !== id) missing.push(id);
    }
    expect(missing).toEqual([]);
    expect(ran.length).toBe(ids.length);
    await settle();
  });

  it('nothing runs a command except the registry', () => {
    const offenders = sources()
      .filter((f) => f.path !== 'extensions/command-registry.ts')
      .filter((f) => /\b(command|cmd)\.run\(\s*(this\.)?(context|ctx)\b/.test(f.text) || /IMT_COMMANDS[\s\S]*\.run\(\s*(this\.)?(context|ctx)\b/.test(f.text))
      .map((f) => f.path);
    expect(offenders).toEqual([]);
  });

  it('every backend of the app is behind the shared proxy', async () => {
    // All backend ports: `export abstract class XxxBackend` anywhere in the app.
    const ports = sources()
      .flatMap((f) => [...f.text.matchAll(/export abstract class (\w+Backend)\b/g)].map((m) => ({ name: m[1] ?? '', path: f.path })))
      .sort((a, b) => a.name.localeCompare(b.name));
    expect(ports.length).toBeGreaterThan(10);
    TestBed.configureTestingModule({ providers: [...appConfig.providers, { provide: Logger, useFactory: () => new MemoryLogger() }] });
    const untracked: string[] = [];
    for (const port of ports) {
      // Resolve the class by importing its module dynamically is not possible here; look it up by name among
      // what the app provides instead.
      const token = BACKEND_TOKENS[port.name];
      if (!token) {
        untracked.push(`${port.name} (${port.path}): add it to BACKEND_TOKENS in this test`);
        continue;
      }
      const instance = TestBed.inject(token, null);
      if (instance !== null && !isActivityTracked(instance)) untracked.push(port.name);
    }
    expect(untracked).toEqual([]);
    await settle();
  });

  it('every backend method is attributed or becomes an operation', { timeout: 20000 }, async () => {
    // Every method name of every backend implementation (mock and real) in the app sources.
    const names = new Set<string>();
    for (const f of sources().filter((s) => /(backend|Backend)/.test(s.path))) {
      for (const m of f.text.matchAll(/^\s{2}(?:async\s+)?(\w+)(?:<[^>]*>)?\([^)]*\)\s*(?::[^{=]+)?\{/gm)) {
        const name = m[1];
        if (name !== undefined && !['constructor', 'if', 'for', 'while', 'switch', 'catch', 'function'].includes(name)) names.add(name);
      }
    }
    expect(names.size).toBeGreaterThan(100);
    TestBed.configureTestingModule({ providers: [{ provide: Logger, useFactory: () => new MemoryLogger() }] });
    const logger = TestBed.inject(Logger) as MemoryLogger;
    // A fake with every name, behind the real proxy.
    const target = Object.fromEntries([...names].map((n) => [n, () => Promise.resolve(n)]));
    const proxied = withFailureLogging(target, logger);
    const silent: string[] = [];
    for (const name of names) {
      const kind = methodKind(name);
      activity.reset();
      activity.setSink((level, message, props) => logger[level]('IMT.Activity', message, props));
      logger.entries.length = 0;
      // Outside any operation: a change is an operation of its own; anything else is not a user operation.
      activity.triggers.note('button');
      await must((proxied as Record<string, () => Promise<unknown>>)[name])();
      const logged = logger.entries.filter((e) => e.category === 'IMT.Activity').length;
      if (kind === 'mutation' && logged !== 1) silent.push(`${name}: mutation without an entry`);
      if (kind !== 'mutation' && logged !== 0) silent.push(`${name}: ${kind} logged as an operation`);
      // Inside an operation: attributed to it, no extra entry.
      await activity.track('outer', 'menu', {}, () => must((proxied as Record<string, () => Promise<unknown>>)[name])());
      const inside = logger.entries.filter((e) => e.category === 'IMT.Activity').length - logged;
      if (inside !== 1) silent.push(`${name}: not attributed to the running operation`);
    }
    expect(silent).toEqual([]);
  });

  it('every Tauri command goes through the operation-id adapter', () => {
    const direct = sources()
      .filter((f) => f.path !== 'core/activity/ipc.ts' && /from '@tauri-apps\/api\/core'|import\('@tauri-apps\/api\/core'\)/.test(f.text) && /\binvoke\b/.test(f.text))
      .map((f) => f.path)
      .filter((p) => !PENDING_DIRECT_INVOKE.has(p));
    expect(direct).toEqual([]);
  });
});

/** Backend ports by name (the guard fails with a hint when a new one is missing here). */
import { AiBackend } from '../../ai/ai-backend';
import { AnalysisBackend } from '../analysis-backend';
import { ImtBackend } from '../backend';
import { CollectBackend, EventLogBackend, LifecycleBackend, PerfmonBackend, PluginPackagesBackend, ProjectBackend, ToolCatalogBackend, ToolsBackend } from '../tools-backend';
import { ProblemsBackend } from '../../problems/problems-backend';
import { StorePaneBackend } from '../../store-pane/store-pane-backend';
import { ConfigDiffBackend } from '../../config-diff/config-diff-backend';
import { PopoutBackend } from '../../popout/popout-backend';
import { ProfilingBackend } from '../../profiling/profiling-backend';
import { DotnetBackend } from '../../profiling/dotnet/dotnet-backend';
import { NativeBackend } from '../../profiling/native/native-backend';
import { DumpsBackend } from '../../dumps/dumps-backend';
import { SchedulesBackend } from '../../schedules/schedules-backend';
import { SettingsBackend } from '../../settings/settings-backend';
import { StorageBackend } from '../../settings/storage/storage-backend';
import { ParsersBackend } from '../../settings/parsers/parsers-backend';
import { ShareBackend } from '../../share/share-backend';
import { SkewBackend } from '../skew-backend';
import { FilesBackend } from '../files-backend';
import { RunsBackend } from '../../timeline/runs-backend';
import { FolderSetupBackend } from '../../ai/folder-setup/folder-setup-backend';
import { BrowserBackend, ChatBrowserBackend } from '../../browser/browser-backend';
import { SnapshotBackend } from '../../diagnostics/snapshot-backend';
import { UpdatesBackend } from '../../updates/updates-backend';
import { UxBackend } from '../ux-backend';
import { WindowingBackend } from '../../windowing/windowing-backend';
import { RecentBackend } from '../recent/recent-backend';
import { JobsBackend } from '../../jobs/jobs-backend';
import { NotificationsBackend } from '../../notifications/notifications-backend';
import { KnowledgeBackend } from '../../ai/knowledge/knowledge-backend';
import { NotesBackend } from '../../notes/notes-backend';
import { DragGhostBackend } from '../../popout/drag-ghost-backend';
import { at, must } from '../../../testing/must';

const BACKEND_TOKENS: Record<string, AbstractType<unknown>> = {
  AiBackend,
  AnalysisBackend,
  ImtBackend,
  BrowserBackend,
  ChatBrowserBackend,
  CollectBackend,
  EventLogBackend,
  FolderSetupBackend,
  LifecycleBackend,
  PluginPackagesBackend,
  PerfmonBackend,
  ProjectBackend,
  ProblemsBackend,
  StorePaneBackend,
  ToolCatalogBackend,
  ToolsBackend,
  ConfigDiffBackend,
  PopoutBackend,
  ProfilingBackend,
  DotnetBackend,
  NativeBackend,
  DumpsBackend,
  SchedulesBackend,
  SettingsBackend,
  StorageBackend,
  ParsersBackend,
  ShareBackend,
  SkewBackend,
  FilesBackend,
  RunsBackend,
  SnapshotBackend,
  UxBackend,
  UpdatesBackend,
  WindowingBackend,
  RecentBackend,
  JobsBackend,
  NotificationsBackend,
  KnowledgeBackend,
  NotesBackend,
  DragGhostBackend,
};
