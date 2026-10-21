import { PageKey } from './browser.types';

/**
 * Replays one of IMT's shortcuts typed while the page had the keyboard (the shell took it from the page): a key press
 * on IMT's focused element, bubbling to the document where IMT's shortcuts listen, as if it had been typed in IMT.
 */
export function replayShortcut(doc: Document, k: Extract<PageKey, { action: 'shortcut' }>): boolean {
  const target = doc.activeElement ?? doc.body;
  const init: KeyboardEventInit = { key: k.key, ctrlKey: k.ctrl, shiftKey: k.shift, altKey: k.alt, metaKey: k.meta, bubbles: true, cancelable: true };
  return target.dispatchEvent(new KeyboardEvent('keydown', init));
}
