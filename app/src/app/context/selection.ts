import { MessageSelection } from './record-actions';

/**
 * The part of `container`'s text that `range` covers, with offsets into the container's text.
 * Null for a collapsed or whitespace-only range.
 */
export function selectionWithin(container: HTMLElement, range: Range): MessageSelection | null {
  const text = range.toString();
  if (!text.trim()) return null;
  const before = document.createRange();
  before.selectNodeContents(container);
  before.setEnd(range.startContainer, range.startOffset);
  const start = before.toString().length;
  return { text, start, end: start + text.length };
}
