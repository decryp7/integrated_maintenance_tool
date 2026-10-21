import { Injectable, InjectionToken } from '@angular/core';

/**
 * A feature's claim on files dropped on the window: it opens the paths it handles and resolves to
 * those paths. Whatever no handler claims is opened as logs, so no file is imported twice.
 */
export type DroppedFilesHandler = (paths: string[]) => Promise<readonly string[]>;

/** Multi token: features (e.g. profiling) contribute handlers; the workspace routes every drop through them. */
export const DROPPED_FILES_HANDLERS = new InjectionToken<readonly DroppedFilesHandler[]>('DROPPED_FILES_HANDLERS');

/**
 * Offers the dropped paths to each handler in turn and resolves to the paths none claimed. A
 * handler that fails claims nothing (it reports its own failures), so the files still open as logs.
 */
export async function routeDroppedFiles(handlers: readonly DroppedFilesHandler[], paths: readonly string[]): Promise<string[]> {
  let rest = [...paths];
  for (const handler of handlers) {
    if (!rest.length) break;
    try {
      const claimed = new Set(await handler(rest));
      rest = rest.filter((p) => !claimed.has(p));
    } catch {
      // Nothing claimed.
    }
  }
  return rest;
}

/** Where a drop landed, in CSS pixels of the window (the OS drop reports physical pixels). */
export interface DropPoint {
  x: number;
  y: number;
}

/**
 * The kinds of drop targets. A target declares what it takes, so the router never guesses from a file's name where it
 * should go: the target's `accept` claims only what it could really open (the AI's images are recognised by their
 * content, in Rust), and everything else goes on to the features and the logs.
 */
export type DropTargetKind = 'aiImages';

/** An element of the page that takes OS file drops landing on it. */
export interface DropTarget {
  kind: DropTargetKind;
  element: Element;
  accept: DroppedFilesHandler;
}

/** The drop targets on screen (components register theirs while they are shown). */
@Injectable({ providedIn: 'root' })
export class DropTargets {
  private readonly targets = new Set<DropTarget>();

  /** Registers `target`; returns its removal. */
  register(target: DropTarget): () => void {
    this.targets.add(target);
    return () => this.targets.delete(target);
  }

  /** The innermost registered target under `point`, if any. */
  at(point: DropPoint, doc: Pick<Document, 'elementFromPoint'> = document): DropTarget | null {
    const hit = doc.elementFromPoint(point.x, point.y);
    if (!hit) return null;
    let best: DropTarget | null = null;
    for (const t of this.targets) {
      if (t.element.contains(hit) && (!best || best.element.contains(t.element))) best = t;
    }
    return best;
  }
}

/**
 * Routes one OS drop: the target under the drop point first (when there is one), then the feature handlers; resolves
 * to the paths nobody claimed (they open as logs).
 */
export async function routeDrop(
  targets: DropTargets | null,
  handlers: readonly DroppedFilesHandler[],
  paths: readonly string[],
  point?: DropPoint | null,
): Promise<string[]> {
  const target = point && targets ? targets.at(point) : null;
  return routeDroppedFiles(target ? [target.accept, ...handlers] : handlers, paths);
}
