import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { FakeContext, provideAnalysisTesting } from '../../analysis/test-setup';
import { MockAiBackend } from '../../core/mock/mock-ai-backend';
import { JobsBackend } from '../../jobs/jobs-backend';
import { JobsStore } from '../../jobs/jobs.store';
import { EMPTY_JOBS, JobInfo } from '../../jobs/jobs.types';
import { el, settle } from '../../../testing/helpers';
import { must } from '../../../testing/must';
import { AiBackend } from '../ai-backend';
import { AiChatPanel } from './chat-panel';
import { ChatStore } from './chat.store';

function queuedJob(sessionId: string, position: number | null, state: JobInfo['state'] = 'queued'): JobInfo {
  return {
    id: 1,
    kind: 'ai',
    title: 'Chat turn',
    source: { window: 'main', link: { type: 'chat', sessionId } },
    state,
    progress: { fraction: null, done: null, total: null, etaMs: null, unit: 'items' },
    message: null,
    queuedAt: 0,
    startedAt: null,
    finishedAt: null,
    elapsedMs: 0,
    etaMs: null,
    cancellable: true,
    retryable: false,
    movable: true,
    position,
    resultType: null,
    error: null,
    unseen: false,
    retryOf: null,
  };
}

async function setup() {
  const backend = new MockAiBackend({ stepMs: 0 });
  const jobs = {
    list: async () => EMPTY_JOBS,
    onChange: async () => () => undefined,
  };
  TestBed.configureTestingModule({
    providers: [...provideAnalysisTesting({ context: new FakeContext() }), { provide: AiBackend, useValue: backend }, { provide: JobsBackend, useValue: jobs }],
  });
  const f = TestBed.createComponent(AiChatPanel);
  const chat = TestBed.inject(ChatStore);
  f.detectChanges();
  for (let i = 0; i < 100 && !chat.activeId(); i++) {
    await settle();
    f.detectChanges();
  }
  vi.spyOn(backend, 'chatSend').mockResolvedValue(1);
  await chat.send('What happened?');
  f.detectChanges();
  return { f, chat, jobs: TestBed.inject(JobsStore), root: f.nativeElement as HTMLElement };
}

describe('AiChatPanel queued turn', () => {
  afterEach(() => TestBed.resetTestingModule());

  it('says Thinking while the turn runs and nothing is queued', async () => {
    const { root } = await setup();
    expect(root.querySelector('[data-testid=ai-chat-queued]')).toBeNull();
    expect(root.querySelector('.thinking')?.getAttribute('aria-label')).toBe('Thinking');
  });

  it('shows the queue position while the chat waits behind another AI request', async () => {
    const { f, chat, jobs, root } = await setup();
    const id = must(chat.activeId());
    jobs.apply({ jobs: [queuedJob(id, 2)], paused: [] });
    f.detectChanges();
    const label = el(f, '[data-testid=ai-chat-queued]');
    expect(label.getAttribute('role')).toBe('status');
    expect(label.textContent).toBe('Queued (position 2) · the AI answers one request at a time');
    expect(root.querySelector('[aria-label=Thinking]')).toBeNull();

    jobs.apply({ jobs: [queuedJob(id, 1)], paused: [] });
    f.detectChanges();
    expect(el(f, '[data-testid=ai-chat-queued]').textContent).toContain('Queued (position 1)');
  });

  it('shows a plain Queued when the position is unknown and goes back to Thinking when the turn starts', async () => {
    const { f, chat, jobs, root } = await setup();
    const id = must(chat.activeId());
    jobs.apply({ jobs: [queuedJob(id, null)], paused: [] });
    f.detectChanges();
    expect(el(f, '[data-testid=ai-chat-queued]').textContent).toBe('Queued · the AI answers one request at a time');
    jobs.apply({ jobs: [queuedJob(id, null, 'running')], paused: [] });
    f.detectChanges();
    expect(root.querySelector('[data-testid=ai-chat-queued]')).toBeNull();
    expect(root.querySelector('[aria-label=Thinking]')).toBeTruthy();
  });

  it('ignores the queued turn of another chat', async () => {
    const { f, jobs, root } = await setup();
    jobs.apply({ jobs: [queuedJob('some-other-chat', 1)], paused: [] });
    f.detectChanges();
    expect(root.querySelector('[data-testid=ai-chat-queued]')).toBeNull();
  });
});
