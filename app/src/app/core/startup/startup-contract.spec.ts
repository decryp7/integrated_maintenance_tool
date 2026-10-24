import { ChangeDetectionStrategy, Component } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { ON_RECOVERED, provideStartupContract, ReadyReply, reportReady, retryUntilAnswered } from './startup-contract';
import { recoveryNotice } from '../../lifecycle/recovery-notice';

/** A call whose attempts are answered by hand. */
function manual<T>() {
  const pending: { resolve: (v: T) => void; reject: (e: unknown) => void }[] = [];
  const call = vi.fn(() => new Promise<T>((resolve, reject) => pending.push({ resolve, reject })));
  return { call, pending };
}

/** Timers run by hand (no sleeps). */
function timers() {
  const queue: (() => void)[] = [];
  return { setTimer: (fn: () => void) => void queue.push(fn), fire: () => queue.shift()?.(), queue };
}

describe('retryUntilAnswered', () => {
  it('answers with the first reply and sends nothing more', async () => {
    const m = manual<number>();
    const t = timers();
    const result = retryUntilAnswered(m.call, { timeoutMs: 10, attempts: 3, setTimer: t.setTimer });
    m.pending[0]!.resolve(7);
    await expect(result).resolves.toBe(7);
    t.fire();
    expect(m.call).toHaveBeenCalledTimes(1);
  });

  it('sends again when an attempt goes unanswered (a lost first IPC), up to the limit', async () => {
    const m = manual<number>();
    const t = timers();
    const result = retryUntilAnswered(m.call, { timeoutMs: 10, attempts: 3, setTimer: t.setTimer });
    t.fire();
    t.fire();
    t.fire();
    expect(m.call).toHaveBeenCalledTimes(3);
    expect(t.queue).toHaveLength(0);
    // The second attempt answers; the late first one does not change the result.
    m.pending[1]!.resolve(2);
    m.pending[0]!.resolve(1);
    await expect(result).resolves.toBe(2);
  });

  it('keeps waiting without a deadline after the last attempt (a paused webview resumes later)', async () => {
    const m = manual<string>();
    const t = timers();
    let done = false;
    const result = retryUntilAnswered(m.call, { timeoutMs: 10, attempts: 2, setTimer: t.setTimer }).then((v) => ((done = true), v));
    t.fire();
    await Promise.resolve();
    expect(done).toBe(false);
    m.pending[0]!.resolve('late');
    await expect(result).resolves.toBe('late');
  });

  it('rejects on the first error reply (the backend answered) and on a call that throws', async () => {
    const m = manual<number>();
    const t = timers();
    const result = retryUntilAnswered(m.call, { timeoutMs: 10, attempts: 3, setTimer: t.setTimer });
    m.pending[0]!.reject(new Error('no'));
    await expect(result).rejects.toThrow('no');
    const thrown = retryUntilAnswered(() => {
      throw new Error('sync');
    }, { timeoutMs: 10, attempts: 1, setTimer: t.setTimer });
    await expect(thrown).rejects.toThrow('sync');
  });
});

describe('reportReady', () => {
  const fast = { timeoutMs: 1, attempts: 1 };

  it('tells the window why Debrief reloaded it', async () => {
    const onRecovered = vi.fn();
    await reportReady(async () => ({ recovered: 'terminated' }), onRecovered, fast);
    expect(onRecovered).toHaveBeenCalledWith('terminated');
    onRecovered.mockClear();
    await reportReady(async () => ({ recovered: null }), onRecovered, fast);
    expect(onRecovered).not.toHaveBeenCalled();
  });

  it('never throws when the report fails (the Rust watchdog acts instead)', async () => {
    await expect(reportReady(() => Promise.reject(new Error('down')), vi.fn(), fast)).resolves.toBeNull();
  });
});

@Component({ selector: 'debrief-ready-host', changeDetection: ChangeDetectionStrategy.OnPush, template: `<p>frame</p>` })
class Host {}

describe('provideStartupContract', () => {
  it('reports ready after the first render, not before, and hands on the recovery reason', async () => {
    let answer!: (r: ReadyReply) => void;
    const send = vi.fn(() => new Promise<ReadyReply>((resolve) => (answer = resolve)));
    const onRecovered = vi.fn();
    TestBed.configureTestingModule({ providers: [provideStartupContract(send), { provide: ON_RECOVERED, useValue: onRecovered }] });
    const fixture = TestBed.createComponent(Host);
    await fixture.whenStable();
    expect(fixture.nativeElement.textContent).toContain('frame');
    expect(send).toHaveBeenCalledTimes(1);
    answer({ recovered: 'unresponsive' });
    await vi.waitFor(() => expect(onRecovered).toHaveBeenCalledWith('unresponsive'));
  });

  it('sends nothing in the browser build', async () => {
    TestBed.configureTestingModule({ providers: [provideStartupContract(null)] });
    const fixture = TestBed.createComponent(Host);
    await fixture.whenStable();
    expect(fixture.nativeElement.textContent).toContain('frame');
  });
});

describe('recoveryNotice', () => {
  it('says calmly what happened and what was kept', () => {
    expect(recoveryNotice('terminated')).toMatch(/reloaded\. Your open files are kept\./);
    expect(recoveryNotice('unresponsive')).toMatch(/did not finish starting/);
  });
});
