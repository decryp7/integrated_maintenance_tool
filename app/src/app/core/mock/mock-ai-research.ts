import { Citation } from '../../ai/ai.types';
import { MockJobCtx, mockJobs, mockSteps, MockJobCancelled } from '../../jobs/mock-jobs';
import { JobResult } from '../../jobs/jobs.types';
import { mockWeb } from '../../browser/mock-browser-backend';

/** What the mock research found (the engine's `ResearchReport`). */
export interface MockResearchReport {
  answer: string;
  sources: { title: string; url: string }[];
}

const PAGES = [
  { title: 'Access denied (0x80070005) in IIS - Microsoft Learn', url: 'https://learn.microsoft.com/troubleshoot/iis/http-error-401-access-denied', ok: true },
  { title: 'Fixing E_ACCESSDENIED for an app pool identity - Stack Overflow', url: 'https://stackoverflow.com/questions/80070005', ok: true },
  { title: 'IIS forum thread', url: 'https://forums.iis.net/t/1234', ok: false },
  { title: 'Windows error codes reference', url: 'https://learn.microsoft.com/windows/win32/debug/system-error-codes', ok: true },
  { title: 'Blog: IIS permissions explained', url: 'https://blog.example.org/iis-permissions', ok: true },
];

/** Questions the mock answers with research: error codes, "what does … mean", "search", "look up". */
export function wantsResearch(question: string): boolean {
  return /0x[0-9a-f]{6,8}|hresult|what does .* mean|look (it )?up|search the web|research/i.test(question);
}

/** Workspace data the mock leak guard flags: the sample hosts. */
export function mockLeaks(text: string): boolean {
  return /\bWEB0\d\b/i.test(text);
}

/** Runs a mock research inside a job's work ("page 2 of 3"), honouring Cancel. */
export async function runMockResearch(ctx: MockJobCtx, question: string, maxResults: number, stepMs: number): Promise<MockResearchReport> {
  // Research holds the browser while it reads (the shell's "AI is browsing"); finished, failed or cancelled, it lets go.
  const web = mockWeb();
  web.drive({ kind: 'research' });
  try {
    return await readPages(ctx, question, maxResults, stepMs);
  } finally {
    if (web.driver?.kind === 'research') web.drive(null);
  }
}

async function readPages(ctx: MockJobCtx, question: string, maxResults: number, stepMs: number): Promise<MockResearchReport> {
  const pages = PAGES.slice(0, Math.max(1, Math.min(10, maxResults)));
  ctx.message('searching');
  await mockSteps(ctx, stepMs, 2);
  const read: { title: string; url: string }[] = [];
  const skipped: string[] = [];
  for (const [i, p] of pages.entries()) {
    if (ctx.cancelled()) throw new MockJobCancelled();
    ctx.message(`page ${i + 1} of ${pages.length}`);
    ctx.progress({ fraction: (i + 1) / pages.length, done: i + 1, total: pages.length });
    await mockSteps(ctx, stepMs, 2);
    if (p.ok) read.push({ title: p.title, url: p.url });
    else skipped.push(`- [${p.title}](${p.url}) — skipped: a login or password field`);
  }
  ctx.message('writing the answer');
  const cite = (n: number) => (read.length >= n ? ` [${n}]` : '');
  const answer = [
    `**HRESULT 0x80070005** is E_ACCESSDENIED: the process asking for the resource has no right to it${cite(1)}. For "${question.slice(0, 80)}", in IIS this usually means the application pool identity cannot read the site's folder or a key${cite(2)}.`,
    '',
    'Likely causes and fixes:',
    `1. Grant the app pool identity (IIS AppPool\\<pool>) read access to the site folder${cite(1)}.`,
    `2. Check that the configured identity's password has not expired${cite(2)}.`,
    `3. Re-run the deployment with a user that can set the folder's permissions${cite(3)}.`,
    '',
    '**Read:**',
    ...read.map((r, i) => `${i + 1}. [${r.title}](${r.url})`),
    ...(skipped.length ? ['', '**Skipped:**', ...skipped] : []),
  ].join('\n');
  return { answer, sources: read };
}

/** The web citations of a report, as the chat shows them. */
export function webCitations(r: MockResearchReport): Citation[] {
  return r.sources.map((s) => ({ kind: 'web', title: s.title, url: s.url }));
}

/** Starts a mock research job that hands its report to `done` (the chat records it); resolves with the job id. */
export function startMockResearch(question: string, maxResults: number, stepMs: number, sessionId: string | null, done: (r: MockResearchReport) => string): number {
  const job = mockJobs().start(
    { kind: 'research', title: `Research: ${question.slice(0, 60)}`, link: sessionId ? { type: 'chat', sessionId } : null },
    async (ctx) => {
      const report = await runMockResearch(ctx, question, maxResults, stepMs);
      const sid = done(report);
      const result: JobResult = { type: 'chat', sessionId: sid };
      return { value: report, result };
    },
  );
  job.done.catch(() => undefined);
  return job.id;
}
