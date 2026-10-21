import { CdkListbox, CdkOption } from '@angular/cdk/listbox';
import { ChangeDetectionStrategy, Component, computed, inject, Injectable, OnInit, signal } from '@angular/core';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { nativePickOne } from '../core/native-dialog';
import { isTauri } from '../core/platform';
import { ToolCatalogBackend } from '../core/tools-backend';
import { TOOL_IDS } from '../core/tools.types';
import { errorText } from '../analysis/async-resource';
import { ToolError } from '../tools/elevation-hint';
import { ToolUnavailable } from '../tools/tool-unavailable';
import { ToolCatalog } from '../tools/tool-catalog.store';
import { DUMPS_STYLES, formatBytes } from './dumps-shared';
import { DumpsStore } from './dumps.store';

/** A process the capture tool lists. */
export interface CaptureProcess {
  pid: number;
  name: string;
  parent: number;
  threads: number;
}

/** What the capture tool wrote (`file` is the tool output the window may open). */
export interface CapturedDump {
  file: string;
  bytes: number;
}

export type CaptureKind = 'mini' | 'heap' | 'full';

/** The `dumps.capture` tool (ADR 0023), reached through the generic `tool_invoke`. */
@Injectable({ providedIn: 'root' })
export class DumpCapture {
  private readonly tools = inject(ToolCatalogBackend);

  processes(): Promise<CaptureProcess[]> {
    return this.tools.toolInvoke<CaptureProcess[]>(TOOL_IDS.dumpCapture, 'processes', {});
  }

  create(pid: number, kind: CaptureKind, folder: string): Promise<CapturedDump> {
    return this.tools.toolInvoke<CapturedDump>(TOOL_IDS.dumpCapture, 'create', { pid, kind, folder });
  }
}

/** Chooses the output folder: the native dialog in Tauri (which grants it), a fixed folder in a browser. */
@Injectable({ providedIn: 'root' })
export class DumpFolderPicker {
  pick(): Promise<string | null> {
    /* v8 ignore next 3: the native dialog exists only inside Tauri */
    if (isTauri()) {
      return nativePickOne({ kind: 'openFolder', title: $localize`:@@dumps.capture.folderTitle:Folder for the dump` });
    }
    return Promise.resolve('C:\\Dumps');
  }
}

/** Filters processes by name or pid. Pure. */
export function filterProcesses(list: readonly CaptureProcess[], filter: string): CaptureProcess[] {
  const f = filter.trim().toLowerCase();
  return f ? list.filter((p) => p.name.toLowerCase().includes(f) || String(p.pid).includes(f)) : [...list];
}

/**
 * Tools ▸ Create dump of process…: pick a running process, the dump type and a folder; the plugin writes the dump
 * with MiniDumpWriteDump (the process is only read) and IMT opens it.
 */
@Component({
  selector: 'imt-create-dump-view',
  changeDetection: ChangeDetectionStrategy.OnPush,
  imports: [CdkListbox, CdkOption, ToolError, ToolUnavailable],
  template: `
    <div class="bar">
      <span class="section-title" i18n="@@dumps.capture.title">Create dump of process</span>
      <span class="spacer"></span>
      <button class="btn ghost" type="button" (click)="load()" [disabled]="loading()" i18n="@@dumps.capture.refresh">Refresh</button>
    </div>
    <div class="body">
      <imt-tool-unavailable [tool]="toolId" />
      @if (unsupported(); as reason) {
        <p class="faint" role="status" data-testid="capture-unsupported">{{ reason }}</p>
      } @else {
        <p class="faint" i18n="@@dumps.capture.intro">The process is only read through the operating system (MiniDumpWriteDump); nothing is injected into it. A full dump holds all of its memory and can be large.</p>
        <imt-tool-error [message]="error()" [retry]="!!error()" (retried)="load()" (dismissed)="error.set(null)" />
        <div class="row-wrap">
          <div class="procs">
            <input type="search" [value]="filter()" (input)="filter.set(value($event))" placeholder="Filter by name or pid" i18n-placeholder="@@dumps.capture.filter" aria-label="Filter processes" i18n-aria-label="@@dumps.capture.filterLabel" />
            <ul cdkListbox class="list" aria-label="Processes" i18n-aria-label="@@dumps.capture.processes" data-testid="capture-processes" [cdkListboxValue]="pidValue()" (cdkListboxValueChange)="pickPid($event.value[0])">
              @for (p of shown(); track p.pid) {
                <li [cdkOption]="p.pid" class="proc"><span class="mono">{{ p.pid }}</span> <span class="ellipsis">{{ p.name }}</span> <span class="faint" i18n="@@dumps.capture.threads">{{ p.threads }} threads</span></li>
              } @empty {
                <li class="faint" role="status">@if (loading()) { <span i18n="@@dumps.capture.loading">Loading…</span> } @else { <span i18n="@@dumps.capture.none">No processes.</span> }</li>
              }
            </ul>
          </div>
          <div class="opts">
            <fieldset>
              <legend i18n="@@dumps.capture.kind">Dump type</legend>
              @for (k of kinds; track k.id) {
                <label class="inline"><input type="radio" name="capture-kind" [checked]="kind() === k.id" (change)="kind.set(k.id)" [attr.data-testid]="'capture-kind-' + k.id" /> {{ k.label }}</label>
                <p class="faint hint">{{ k.hint }}</p>
              }
            </fieldset>
            <div class="folder">
              <button class="btn" type="button" (click)="chooseFolder()" data-testid="capture-folder" i18n="@@dumps.capture.chooseFolder">Choose folder…</button>
              <span class="mono ellipsis">{{ folder() ?? '' }}</span>
            </div>
            <button class="btn primary" type="button" data-testid="capture-create" [disabled]="!canCreate()" (click)="create()">
              @if (busy()) { <span i18n="@@dumps.capture.busy">Writing the dump…</span> } @else { <span i18n="@@dumps.capture.create">Create dump</span> }
            </button>
            @if (made(); as m) {
              <p role="status" data-testid="capture-done" i18n="@@dumps.capture.done">Wrote {{ m.file }} ({{ bytes(m.bytes) }}).</p>
            }
          </div>
        </div>
      }
    </div>
  `,
  styles: [
    DUMPS_STYLES,
    `
      .row-wrap { display: flex; gap: 16px; flex-wrap: wrap; margin-top: 8px; }
      .procs { flex: 1; min-width: 260px; display: flex; flex-direction: column; gap: 6px; }
      .procs .list { max-height: 360px; overflow: auto; border: 1px solid var(--border-2); border-radius: 4px; }
      .proc { display: flex; gap: 8px; padding: 3px 8px; cursor: pointer; }
      .proc[aria-selected='true'] { background: var(--bg-sel); }
      .proc:focus-visible { outline: 2px solid var(--focus-ring); outline-offset: -2px; }
      .opts { width: 320px; display: flex; flex-direction: column; gap: 10px; }
      fieldset { border: 1px solid var(--border-2); border-radius: 4px; padding: 6px 10px; margin: 0; }
      legend { color: var(--muted); padding: 0 4px; }
      .hint { margin: 0 0 6px 22px; font-size: var(--fs-xs); }
      .folder { display: flex; gap: 8px; align-items: center; min-width: 0; }
    `,
  ],
})
export class CreateDumpView implements OnInit {
  private readonly capture = inject(DumpCapture);
  private readonly folders = inject(DumpFolderPicker);
  private readonly catalog = inject(ToolCatalog);
  private readonly store = inject(DumpsStore);
  private readonly context = inject(COMMAND_CONTEXT);
  protected readonly toolId = TOOL_IDS.dumpCapture;

  protected readonly kinds: { id: CaptureKind; label: string; hint: string }[] = [
    { id: 'mini', label: $localize`:@@dumps.capture.mini:Minidump`, hint: $localize`:@@dumps.capture.miniHint:Threads, stacks, modules and handles. Small; enough for most crashes and hangs.` },
    { id: 'heap', label: $localize`:@@dumps.capture.heap:Minidump with heap`, hint: $localize`:@@dumps.capture.heapHint:Adds the heap and data, for inspecting objects in a debugger.` },
    { id: 'full', label: $localize`:@@dumps.capture.full:Full dump`, hint: $localize`:@@dumps.capture.fullHint:All memory of the process: as large as its working set.` },
  ];
  readonly processes = signal<CaptureProcess[]>([]);
  readonly filter = signal('');
  readonly pid = signal<number | null>(null);
  readonly kind = signal<CaptureKind>('mini');
  readonly folder = signal<string | null>(null);
  readonly loading = signal(false);
  readonly busy = signal(false);
  readonly error = signal<string | null>(null);
  readonly made = signal<CapturedDump | null>(null);

  protected readonly shown = computed(() => filterProcesses(this.processes(), this.filter()));
  /** The listbox's value: the chosen process while it is listed. */
  protected readonly pidValue = computed(() => {
    const pid = this.pid();
    return pid !== null && this.shown().some((p) => p.pid === pid) ? [pid] : [];
  });
  protected readonly canCreate = computed(() => this.pid() !== null && this.folder() !== null && !this.busy() && this.catalog.usable(this.toolId));
  /** Why the tool cannot run on this platform (the engine's reason). */
  protected readonly unsupported = computed(() => {
    const a = this.catalog.entry(this.toolId)?.availability;
    return a?.state === 'unsupported' ? a.reason : null;
  });

  ngOnInit(): void {
    void this.load();
  }

  async load(): Promise<void> {
    if (this.unsupported()) return;
    this.loading.set(true);
    try {
      this.processes.set(await this.capture.processes());
      this.error.set(null);
    } catch (e) {
      this.error.set(errorText(e));
    } finally {
      this.loading.set(false);
    }
  }

  protected value(e: Event): string {
    return (e.target as HTMLInputElement).value;
  }

  pickPid(v: unknown): void {
    if (typeof v === 'number') this.pid.set(v);
  }

  async chooseFolder(): Promise<void> {
    const f = await this.folders.pick();
    if (f) this.folder.set(f);
  }

  /** Writes the dump and opens it in the Dump view. */
  async create(): Promise<void> {
    const pid = this.pid();
    const folder = this.folder();
    if (pid === null || folder === null) return;
    this.busy.set(true);
    this.error.set(null);
    this.made.set(null);
    try {
      const made = await this.capture.create(pid, this.kind(), folder);
      this.made.set(made);
      const opened = await this.store.openPaths([made.file]);
      if (opened.length) this.context.showView('dump');
    } catch (e) {
      this.error.set(errorText(e));
    } finally {
      this.busy.set(false);
    }
  }

  protected readonly bytes = formatBytes;
}
