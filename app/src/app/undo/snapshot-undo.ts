import { UndoStack } from './undo-stack';

/** How a store's state is read and put back for undo. */
export interface SnapshotTarget<T> {
  read: () => T;
  write: (value: T) => void;
}

/**
 * Makes a local (UI-state) edit undoable by snapshots: reads the state, applies `mutate`, reads it again and,
 * when it changed, records an entry that writes the old or the new snapshot back. States are compared as JSON,
 * so edits that change nothing record nothing.
 */
export function recordSnapshot<T>(undo: UndoStack, target: SnapshotTarget<T>, label: string, mutate: () => void, options: { mergeKey?: string; group?: string } = {}): void {
  const before = target.read();
  mutate();
  const after = target.read();
  if (JSON.stringify(before) === JSON.stringify(after)) return;
  undo.record({ label, mergeKey: options.mergeKey, group: options.group, do: () => target.write(after), undo: () => target.write(before) });
}
