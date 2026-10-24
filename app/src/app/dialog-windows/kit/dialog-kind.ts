/**
 * Dialog kinds (ADR 0054, Paddock ADR 0009). Self-contained and free of app imports: this folder moves to `paddock-ui`
 * as it is. A dialog declares exactly one kind; the kind sets the defaults the shell applies (modality, size, footer,
 * keys), so a dialog states only what differs.
 */
export const DIALOG_KINDS = ['decision', 'form', 'viewer', 'tool'] as const;
export type DialogKind = (typeof DIALOG_KINDS)[number];

/**
 * Whether a dialog blocks the window that opened it. Modal: the user must answer before going on (the parent is
 * disabled and dimmed). Modeless: a tool kept beside the work (the parent stays usable).
 */
export type DialogModality = 'modal' | 'modeless';

/** What the shell draws when the content has no Close/Cancel of its own, and what Escape does. */
export type DialogDismiss = 'cancel' | 'close';

/** A window size in logical px. */
export interface DialogSize {
  readonly width: number;
  readonly height: number;
}

/** What a kind fixes, independent of any one dialog. */
export interface KindPolicy {
  /** The modalities the kind allows; one entry means the kind decides, two means the dialog declares it. */
  readonly modalities: readonly DialogModality[];
  /** The smallest window of the kind (the viewer floor), or none. */
  readonly minSize: DialogSize | null;
  /** The size the window opens at when the dialog declares none; `null` height means "fitted to its content". */
  readonly defaultSize: { readonly width: number | null; readonly height: number | null };
  /** The window is fitted to its content after the first render (grow-only: it never shrinks under a reader). */
  readonly fitsContent: boolean;
  /** A dialog of this kind must declare its width. */
  readonly needsWidth: boolean;
}

/** The viewer floor: results open at about 680x560 and only grow. */
export const VIEWER_MIN: DialogSize = { width: 680, height: 560 };
/** A tool's default size. */
export const TOOL_DEFAULT: DialogSize = { width: 960, height: 680 };
/** The least a dialog window can be (matches the native shell). */
export const DIALOG_MIN: DialogSize = { width: 320, height: 200 };

export const KIND_POLICY: Readonly<Record<DialogKind, KindPolicy>> = {
  decision: { modalities: ['modal'], minSize: null, defaultSize: { width: null, height: null }, fitsContent: true, needsWidth: true },
  form: { modalities: ['modal', 'modeless'], minSize: null, defaultSize: { width: null, height: null }, fitsContent: true, needsWidth: true },
  viewer: { modalities: ['modeless'], minSize: VIEWER_MIN, defaultSize: VIEWER_MIN, fitsContent: true, needsWidth: false },
  tool: { modalities: ['modeless'], minSize: null, defaultSize: TOOL_DEFAULT, fitsContent: false, needsWidth: false },
};

/** The modality of a dialog of `kind` that declared `declared` (only a form chooses); throws on a combination the kind forbids. */
export function modalityOf(kind: DialogKind, declared: DialogModality | undefined): DialogModality {
  const allowed = KIND_POLICY[kind].modalities;
  if (declared === undefined) {
    const [only] = allowed;
    if (allowed.length !== 1 || only === undefined) throw new Error(`a ${kind} dialog must declare its modality`);
    return only;
  }
  if (!allowed.includes(declared)) throw new Error(`a ${kind} dialog cannot be ${declared}`);
  return declared;
}

/** What Escape, Cmd/Ctrl+W and a window close request do: a modal's Cancel, a modeless window's Close. */
export function dismissOf(modality: DialogModality): DialogDismiss {
  return modality === 'modal' ? 'cancel' : 'close';
}

/** The resolved window size of a dialog: declared values, then the kind's defaults, never below the kind's floor. */
export interface ResolvedSize {
  readonly width: number;
  /** `undefined`: the window is fitted to its content. */
  readonly height: number | undefined;
  readonly minWidth: number | undefined;
  readonly minHeight: number | undefined;
}

export function resolveSize(
  kind: DialogKind,
  declared: { width?: number; height?: number; minWidth?: number; minHeight?: number },
): ResolvedSize {
  const p = KIND_POLICY[kind];
  const floor = p.minSize;
  const width = Math.max(declared.width ?? p.defaultSize.width ?? 0, floor?.width ?? 0);
  const given = declared.height ?? p.defaultSize.height ?? undefined;
  const height = given === undefined ? undefined : Math.max(given, floor?.height ?? 0);
  const minWidth = declared.minWidth === undefined && !floor ? undefined : Math.max(declared.minWidth ?? 0, floor?.width ?? 0);
  const minHeight = declared.minHeight === undefined && !floor ? undefined : Math.max(declared.minHeight ?? 0, floor?.height ?? 0);
  return { width, height, minWidth, minHeight };
}

/** A remembered size raised to the kind's floor (a remembered viewer smaller than 680x560 is made 680x560). */
export function raiseToKindMinimum<T extends { width: number; height: number }>(kind: DialogKind, bounds: T): T {
  const floor = KIND_POLICY[kind].minSize;
  return floor ? { ...bounds, width: Math.max(bounds.width, floor.width), height: Math.max(bounds.height, floor.height) } : bounds;
}

/**
 * The commands every dialog window needs whatever it shows: taking its input, answering, sizing itself. The kind adds
 * them; a dialog never lists them. (The app adds its own base set: UI sync, logging.)
 */
export const KIT_BASE_COMMANDS: readonly string[] = ['dialog_take_input', 'dialog_close', 'dialog_fit_height'];
