import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend } from '../../testing/helpers';
import { RecordView } from '../core/api.types';
import { ImtBackend } from '../core/backend';
import { EMPTY_QUERY } from '../core/workspace.store';
import { ContextSection } from './context-section';
import { at } from '../../testing/must';

describe('ContextSection', () => {
  async function render(index = 3, backend = tinyBackend()) {
    TestBed.configureTestingModule({ providers: [provideTinyBackend(backend)] });
    const fixture = TestBed.createComponent(ContextSection);
    const rows = (await backend.queryPage({ query: EMPTY_QUERY, offset: 0, limit: 20, descending: false })).rows;
    fixture.componentRef.setInput('record', rows[index]);
    await settle(fixture);
    return { fixture, backend, rows };
  }

  const open = async (fixture: Awaited<ReturnType<typeof render>>['fixture']) => {
    el<HTMLButtonElement>(fixture, 'button.toggle').click();
    await settle(fixture);
  };

  it('is collapsed and does not call the engine until opened', async () => {
    const backend = tinyBackend();
    const spy = vi.spyOn(backend, 'sourceContext');
    const { fixture } = await render(3, backend);
    expect(el(fixture, 'button.toggle').getAttribute('aria-expanded')).toBe('false');
    expect(all(fixture, '.line')).toHaveLength(0);
    expect(spy).not.toHaveBeenCalled();
  });

  it('shows the lines around the event with the event line highlighted', async () => {
    const backend = tinyBackend();
    const spy = vi.spyOn(backend, 'sourceContext');
    const { fixture, rows } = await render(3, backend);
    await open(fixture);
    const rec = at(rows, 3);
    expect(spy).toHaveBeenCalledWith({ recordId: rec.id, before: 10, after: 10 });
    const lines = all(fixture, '.line');
    expect(lines.length).toBeGreaterThan(1);
    const hit = all(fixture, '.line.hit');
    expect(hit).toHaveLength(1);
    expect(at(hit, 0).getAttribute('aria-current')).toBe('true');
    expect(at(hit, 0).textContent).toContain(String(rec.line));
    expect(at(hit, 0).textContent).toContain(rec.message);
    expect(text(fixture)).toContain(rec.sourceName);
    expect(el(fixture, '.lines').classList.contains('mono')).toBe(true);
  });

  it('More expands step by step up to 500 and then stops', async () => {
    const backend = tinyBackend();
    const spy = vi.spyOn(backend, 'sourceContext');
    const { fixture, rows } = await render(3, backend);
    await open(fixture);
    for (const radius of [50, 200, 500]) {
      buttonByText(fixture, 'More').click();
      await settle(fixture);
      expect(spy).toHaveBeenLastCalledWith({ recordId: at(rows, 3).id, before: radius, after: radius });
    }
    expect(buttonByText(fixture, 'More').disabled).toBe(true);
    expect(text(fixture)).toContain('±500 lines');
  });

  it('starts again from the default size for another event', async () => {
    const backend = tinyBackend();
    const spy = vi.spyOn(backend, 'sourceContext');
    const { fixture, rows } = await render(3, backend);
    await open(fixture);
    buttonByText(fixture, 'More').click();
    await settle(fixture);
    fixture.componentRef.setInput('record', rows[5]);
    await settle(fixture);
    expect(spy).toHaveBeenLastCalledWith({ recordId: at(rows, 5).id, before: 10, after: 10 });
    expect(spy.mock.calls.filter((c) => c[0].recordId === at(rows, 5).id && c[0].before !== 10)).toHaveLength(0);
    expect(text(fixture)).toContain('±10 lines');
  });

  it('keeps the section open for the next event', async () => {
    const { fixture, rows } = await render(3);
    await open(fixture);
    fixture.componentRef.setInput('record', rows[4]);
    await settle(fixture);
    expect(at(all(fixture, '.line.hit'), 0).textContent).toContain(String(at(rows, 4).line));
  });

  it('copies the lines as text', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const { fixture } = await render(3);
    await open(fixture);
    buttonByText(fixture, 'Copy').click();
    await settle(fixture);
    const copied = at(writeText.mock.calls, 0)[0] as string;
    expect(copied.split('\n')).toHaveLength(all(fixture, '.line').length);
    expect(buttonByText(fixture, 'Copied')).toBeDefined();
    writeText.mockRejectedValueOnce(new Error('denied'));
    buttonByText(fixture, 'Copied').click();
    await settle(fixture);
    expect(buttonByText(fixture, 'Copy failed')).toBeDefined();
    vi.unstubAllGlobals();
  });

  it('shows an inline error with Retry', async () => {
    const backend = tinyBackend();
    vi.spyOn(backend, 'sourceContext').mockRejectedValueOnce('file is locked');
    const { fixture } = await render(3, backend);
    await open(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('file is locked');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture);
    expect(all(fixture, '[role=alert]')).toHaveLength(0);
    expect(all(fixture, '.line').length).toBeGreaterThan(0);
  });

  it('ignores a slow answer after the section was closed or the event changed', async () => {
    const backend = tinyBackend();
    const real = backend.sourceContext.bind(backend);
    let release!: () => void;
    vi.spyOn(backend, 'sourceContext').mockImplementationOnce(async (r) => {
      const ctx = await real(r);
      await new Promise<void>((res) => (release = res));
      return ctx;
    });
    const { fixture } = await render(3, backend);
    await open(fixture);
    await open(fixture);
    release();
    await settle(fixture);
    expect(all(fixture, '.line')).toHaveLength(0);
    await open(fixture);
    expect(all(fixture, '.line').length).toBeGreaterThan(0);
  });

  it('shows Loading until the engine answers', async () => {
    const backend = tinyBackend();
    vi.spyOn(backend, 'sourceContext').mockReturnValueOnce(new Promise(() => undefined));
    const { fixture } = await render(3, backend);
    await open(fixture);
    expect(el(fixture, '[role=status]').textContent).toContain('Loading');
  });

  it('scrolls the event line to the middle of the box', async () => {
    const { fixture } = await render(3);
    const offsetTop = vi.spyOn(HTMLElement.prototype, 'offsetTop', 'get').mockReturnValue(400);
    const clientHeight = vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(200);
    await open(fixture);
    await settle(fixture);
    expect(el(fixture, '.lines').scrollTop).toBeGreaterThan(0);
    offsetTop.mockRestore();
    clientHeight.mockRestore();
    void (null as unknown as RecordView);
    void (null as unknown as ImtBackend);
  });
});
