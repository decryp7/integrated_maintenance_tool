import { Rect } from '../dock/dock-geometry';
import { WorkArea } from '../popout/bounds';
import { Unlisten } from '../windowing/window-events';
import { DIALOG_CLOSED_EVENT, DIALOG_RESULT_EVENT, DialogInfo, DialogIo, DialogOpenRequest, DialogStart } from './dialog-io';
import { ConfirmQuestion } from './dialog-session';

/** A scripted `DialogIo` for specs: records calls, lets a test play the shell's events. */
export class FakeDialogIo implements DialogIo {
  readonly opened: DialogOpenRequest[] = [];
  readonly closeRequests: string[] = [];
  readonly closedWith: unknown[] = [];
  readonly asked: ConfirmQuestion[] = [];
  private readonly handlers = new Map<string, Set<(p: unknown) => void>>();
  private next = 1;
  /** Labels by dialog id, so opening a dialog again answers `existing`. */
  private readonly labels = new Map<string, string>();
  failOpen: Error | null = null;
  answer = true;
  start: DialogStart = { dialogId: 'x', title: 'X', modality: 'modeless', input: {} };
  areas: WorkArea[] = [];
  label = 'main';

  async open(request: DialogOpenRequest): Promise<DialogInfo> {
    this.opened.push(request);
    if (this.failOpen) throw this.failOpen;
    const known = this.labels.get(request.dialogId);
    const label = known ?? `d-main-${this.next++}`;
    this.labels.set(request.dialogId, label);
    return { label, dialogId: request.dialogId, existing: !!known };
  }
  async take() {
    return this.start;
  }
  async close(result: unknown) {
    this.closedWith.push(result);
  }
  async listen(event: string, handler: (payload: unknown) => void): Promise<Unlisten> {
    const set = this.handlers.get(event) ?? new Set();
    set.add(handler);
    this.handlers.set(event, set);
    return () => set.delete(handler);
  }
  async requestClose(label: string) {
    this.closeRequests.push(label);
  }
  async ask(q: ConfirmQuestion) {
    this.asked.push(q);
    return this.answer;
  }
  async workAreas() {
    return this.areas;
  }
  async windowLabel() {
    return this.label;
  }
  /** Plays an event of the shell. */
  emit(event: string, payload: unknown): void {
    for (const h of this.handlers.get(event) ?? []) h(payload);
  }
  result(label: string, dialogId: string, result: unknown): void {
    this.emit(DIALOG_RESULT_EVENT, { label, dialogId, result });
  }
  closed(label: string, dialogId: string, bounds: Rect | null = null): void {
    this.emit(DIALOG_CLOSED_EVENT, { label, dialogId, bounds });
  }
  listeners(event: string): number {
    return this.handlers.get(event)?.size ?? 0;
  }
}
