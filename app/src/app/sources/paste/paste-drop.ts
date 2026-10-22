import { DestroyRef, inject } from '@angular/core';
import { PasteCommands } from './paste-commands';

/** The data of a drag, as far as it matters here. */
export interface DragData {
  readonly files: { readonly length: number };
  readonly types: readonly string[];
  getData(format: string): string;
}

/** Whether a drag carries plain text and no files (checked while it is over the window, when only the types are known). */
export function dragsText(dt: Pick<DragData, 'files' | 'types'> | null): boolean {
  return !!dt && !dt.files.length && dt.types.includes('text/plain');
}

/** The dropped plain text, or null when the drop carries files or no text. Pure. */
export function droppedText(dt: DragData | null): string | null {
  if (!dt || dt.files.length) return null;
  const text = dt.getData('text/plain');
  return text.trim() ? text : null;
}

/**
 * Dropping plain text on the window opens "Paste as log" with it (files keep their own routing: `routeDrops`). Not
 * dropped on a text field, which takes the text itself.
 */
export function startTextDrops(): void {
  const commands = inject(PasteCommands);
  const target = document;
  const editable = (e: Event) => e.target instanceof Element && !!e.target.closest('input, textarea, [contenteditable]');
  const over = (e: DragEvent) => {
    if (dragsText(e.dataTransfer) && !editable(e)) e.preventDefault();
  };
  const drop = (e: DragEvent) => {
    if (editable(e)) return;
    const text = droppedText(e.dataTransfer);
    if (text === null) return;
    e.preventDefault();
    void commands.open(text);
  };
  target.addEventListener('dragover', over);
  target.addEventListener('drop', drop);
  inject(DestroyRef).onDestroy(() => {
    target.removeEventListener('dragover', over);
    target.removeEventListener('drop', drop);
  });
}
