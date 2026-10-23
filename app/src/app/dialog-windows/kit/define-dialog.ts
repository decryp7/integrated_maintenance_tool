import { InputSignalWithTransform, Injector, Type } from '@angular/core';
import { DIALOG_KINDS, DialogKind, DialogModality, DialogSize, KIND_POLICY, modalityOf, resolveSize } from './dialog-kind';

/** A type carried at compile time only: `type<NarrativeResult>()` states a dialog's result without a value. */
export interface TypeToken<T> {
  readonly __type?: T;
}
export function type<T>(): TypeToken<T> {
  return {};
}

/** What a dialog component declares about itself: its id (and, optionally, whether it may close now). */
export interface DialogComponent {
  readonly dialogId?: string;
  /** Asked before the dialog closes by any route; false keeps it open (unsaved edits). Runs in the dialog's window. */
  dialogCanClose?(injector: Injector): boolean;
}

/** The dialog id a component class declares, if any. */
export function dialogIdOf(component: Type<unknown>): string | undefined {
  const id = (component as unknown as DialogComponent).dialogId;
  return typeof id === 'string' && id ? id : undefined;
}

/** A short stable key of an input value (FNV-1a over its JSON): the identity of an instanced dialog. */
export function instanceKey(inputs: unknown): string {
  let h = 0x811c9dc5;
  for (const ch of JSON.stringify(inputs) ?? '') {
    h ^= ch.charCodeAt(0);
    h = Math.imul(h, 0x01000193) >>> 0;
  }
  return h.toString(36);
}

/** What `defineDialog` takes: the whole contract of one dialog. */
export interface DialogSpec<Id extends string, Backend extends string, C, R> {
  /** Stable id (`ai-setup`): the dialog window's route, its remembered bounds' key, its test id. */
  readonly id: Id;
  readonly kind: DialogKind;
  /** Only a `form` chooses (modal or modeless); every other kind decides. */
  readonly modality?: DialogModality;
  /** Declared size; the kind supplies the rest (see `resolveSize`). */
  readonly width?: number;
  readonly height?: number;
  readonly minWidth?: number;
  readonly minHeight?: number;
  /**
   * Each distinct input is its own dialog (a clock adjustment per source, a narrative per event): opening the same
   * id with the same input focuses it, with another input opens another window. Without it, one window per id.
   */
  readonly instanced?: boolean;
  /** A modal without a Cancel (a choice between two ways forward): Escape, Cmd/Ctrl+W and a close request leave it open. */
  readonly requiredChoice?: boolean;
  /** What the opener's promise resolves to (`type<void>()`: nothing). */
  readonly result: TypeToken<R>;
  /** Provider groups the dialog's window boots besides the small base set. */
  readonly backends?: readonly Backend[];
  /**
   * Every IPC command the dialog's component, and the services it reaches, may call, beyond the kind's and the app's
   * base commands. A trailing `*` is a prefix. The guard in Rust allows exactly these (generated copy: dialog-contracts.json).
   */
  readonly commands: readonly string[];
  /** The content component, loaded when the dialog window opens (the host never loads it). */
  readonly load: () => Promise<Type<C>>;
}

/** A defined dialog: the declared contract plus what the kind derived. `_in` / `_result` carry the types. */
export interface DialogDef<Id extends string = string, Backend extends string = string, C = unknown, R = unknown> {
  readonly id: Id;
  readonly kind: DialogKind;
  readonly modality: DialogModality;
  readonly width: number;
  /** `undefined`: fitted to the content. */
  readonly height?: number;
  readonly minWidth?: number;
  readonly minHeight?: number;
  readonly instanced?: boolean;
  readonly requiredChoice?: boolean;
  readonly backends?: readonly Backend[];
  readonly commands: readonly string[];
  /** The window is fitted to its content, growing only. */
  readonly fit: boolean;
  readonly load: () => Promise<Type<C>>;
  readonly _result?: R;
}

/** Declares one dialog: validates the combination once, here, and derives modality, size and fit from the kind. */
export function defineDialog<const Id extends string, Backend extends string, C, R>(
  spec: DialogSpec<Id, Backend, C, R>,
): DialogDef<Id, Backend, C, R> {
  if (!/^[a-z0-9][a-z0-9-]{0,63}$/.test(spec.id)) throw new Error(`dialog id ${spec.id} is malformed`);
  if (!DIALOG_KINDS.includes(spec.kind)) throw new Error(`${spec.id}: unknown kind ${spec.kind}`);
  const policy = KIND_POLICY[spec.kind];
  if (policy.needsWidth && spec.width === undefined) throw new Error(`${spec.id}: a ${spec.kind} dialog declares its width`);
  const modality = modalityOf(spec.kind, spec.modality);
  if (spec.requiredChoice && modality !== 'modal') throw new Error(`${spec.id}: only a modal can be a required choice`);
  const size = resolveSize(spec.kind, spec);
  // A form or decision that names a height keeps it; one that names none is fitted. A viewer and a tool always have one.
  const fit = size.height === undefined || spec.kind === 'viewer';
  return {
    id: spec.id,
    kind: spec.kind,
    modality,
    width: size.width,
    height: size.height,
    minWidth: size.minWidth,
    minHeight: size.minHeight,
    instanced: spec.instanced,
    requiredChoice: spec.requiredChoice,
    backends: spec.backends,
    commands: spec.commands,
    fit,
    load: spec.load,
  };
}

/** The inputs a component takes, from its `input()` signals: `{ record?: Rec }` for `record = input<Rec>()`. */
export type DialogInputsOf<C> = {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  [K in keyof C as C[K] extends InputSignalWithTransform<any, any> ? K : never]?: C[K] extends InputSignalWithTransform<any, infer W> ? W : never;
};

/** One dialog of a registry as written to the generated contract file read by Rust. */
export interface DialogContract {
  readonly kind: DialogKind;
  readonly modality: DialogModality;
  readonly requiredChoice: boolean;
  readonly instanced: boolean;
  readonly minSize: DialogSize | null;
  readonly commands: readonly string[];
}

/** The file Rust reads (`app/src-tauri/dialog-contracts.json`): the base commands and one contract per dialog id. */
export interface DialogContractFile {
  readonly version: 1;
  readonly base: readonly string[];
  readonly dialogs: Readonly<Record<string, DialogContract>>;
}

/** The contract file of `defs`: sorted by id, commands sorted, so the checked-in copy has a stable diff. */
export function contractFile(defs: readonly DialogDef[], base: readonly string[]): DialogContractFile {
  const dialogs: Record<string, DialogContract> = {};
  for (const d of [...defs].sort((a, b) => a.id.localeCompare(b.id))) {
    dialogs[d.id] = {
      kind: d.kind,
      modality: d.modality,
      requiredChoice: d.requiredChoice === true,
      instanced: d.instanced === true,
      minSize: minSizeOf(d),
      commands: [...d.commands].sort(),
    };
  }
  return { version: 1, base: [...base].sort(), dialogs };
}

function minSizeOf(d: DialogDef): DialogSize | null {
  const floor = KIND_POLICY[d.kind].minSize;
  const width = Math.max(d.minWidth ?? 0, floor?.width ?? 0);
  const height = Math.max(d.minHeight ?? 0, floor?.height ?? 0);
  return width > 0 || height > 0 ? { width, height } : null;
}

/** Whether `command` is covered by a declared entry (an exact name, or a `prefix*`). */
export function commandCovered(declared: readonly string[], command: string): boolean {
  return declared.some((d) => (d.endsWith('*') ? command.startsWith(d.slice(0, -1)) : d === command));
}
