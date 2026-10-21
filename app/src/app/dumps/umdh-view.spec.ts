import { ComponentFixture, TestBed } from '@angular/core/testing';
import { describe, expect, it, vi } from 'vitest';
import { all, buttonByText, el, settle, text, typeInto } from '../../testing/helpers';
import { UmdhTreeNode } from '../core/api.types';
import { CmdError } from '../core/cmd-error';
import { DumpsExplainer } from './dumps-shared';
import { DumpsStore } from './dumps.store';
import { MockDumpsBackend } from './mock/mock-dumps-backend';
import { setupDumps } from './testing/dumps-test-support';
import { insertChildren, TreeItem, UmdhView } from './umdh-view';

async function mount(open = true, backend?: MockDumpsBackend) {
  setupDumps({ backend });
  const explain = vi.fn(async () => true);
  TestBed.overrideProvider(DumpsExplainer, { useValue: { explain } });
  if (open) await TestBed.inject(DumpsStore).pickAndOpen('umdh');
  const fixture: ComponentFixture<UmdhView> = TestBed.createComponent(UmdhView);
  fixture.detectChanges();
  await settle(fixture, 6);
  return { fixture, explain, view: fixture.componentInstance, store: TestBed.inject(DumpsStore) };
}

const tab = (f: ComponentFixture<UmdhView>, id: string) => el<HTMLButtonElement>(f, `[data-testid="umdh-tab-${id}"]`);
const node = (key: string, hasChildren = true): UmdhTreeNode => ({ key, label: key, deltaBytes: 1, deltaCount: 1, hasChildren, framework: false });

describe('insertChildren', () => {
  it('inserts children after their parent and replaces older ones', () => {
    const a: TreeItem = { node: node('a'), path: ['a'], level: 0 };
    const b: TreeItem = { node: node('b'), path: ['b'], level: 0 };
    const once = insertChildren([a, b], a, [node('x'), node('y', false)]);
    expect(once.map((i) => i.path.join('/'))).toEqual(['a', 'a/x', 'a/y', 'b']);
    expect(once[1]!.level).toBe(1);
    const again = insertChildren(once, a, [node('z')]);
    expect(again.map((i) => i.path.join('/'))).toEqual(['a', 'a/z', 'b']);
    expect(insertChildren([b], a, [node('x')])).toEqual([b]);
  });
});

describe('UmdhView', () => {
  it('offers to open UMDH logs when none is open', async () => {
    const { fixture } = await mount(false);
    expect(text(fixture)).toContain('Open UMDH snapshots');
    buttonByText(fixture, 'Open UMDH snapshots…').click();
    await settle(fixture, 8);
    expect(all(fixture, 'li.finding').length).toBeGreaterThan(0);
  });

  it('leads with the leak findings over every snapshot', async () => {
    const { fixture } = await mount();
    expect(all(fixture, 'li.finding').map((f) => f.getAttribute('data-kind'))).toEqual(['leakGrowth', 'steadyGrowth', 'heapTotal']);
    expect(text(fixture)).toContain('Steady growth in Shop.Orders!OrderCache::Add+0x4a across 3 snapshots');
  });

  it('shows the growth grouped three ways and a stack from a finding', async () => {
    const { fixture, view } = await mount();
    el<HTMLButtonElement>(fixture, 'button[aria-label^="Show the evidence of Shop.Orders!OrderCache::Add+0x4a grew"]').click();
    await settle(fixture, 8);
    expect(view.tab()).toBe('growth');
    expect(view.stackKey()).toBe('cache');
    const rows = all(fixture, '[data-testid="umdh-growth"] tbody tr');
    expect(rows[0]!.textContent).toContain('+20 MB');
    expect(el(fixture, '[data-testid="umdh-stack"]').textContent).toContain('allocated here');
    el<HTMLInputElement>(fixture, '[data-testid="umdh-group-module"]').dispatchEvent(new Event('change'));
    await settle(fixture, 6);
    expect(all(fixture, '[data-testid="umdh-growth"] tbody tr')[0]!.textContent).toContain('Shop.Orders');
    expect(fixture.nativeElement.querySelector('[data-testid="umdh-stack"]')).toBeNull();
    el<HTMLInputElement>(fixture, '[data-testid="umdh-group-function"]').dispatchEvent(new Event('change'));
    typeInto(el<HTMLInputElement>(fixture, 'input[type="search"]'), 'report');
    await settle(fixture, 6);
    expect(all(fixture, '[data-testid="umdh-growth"] tbody tr')).toHaveLength(1);
    // A finding while grouped by function switches back to stacks and selects it.
    tab(fixture, 'findings').click();
    await settle(fixture, 6);
    el<HTMLButtonElement>(fixture, 'button[aria-label^="Show the evidence of Steady growth"]').click();
    await settle(fixture, 8);
    expect(view.group()).toBe('stack');
    expect(view.stackKey()).toBe('cache');
  });

  it('selects stack rows and compares other logs', async () => {
    const { fixture, view, store } = await mount();
    tab(fixture, 'growth').click();
    await settle(fixture, 6);
    const rows = all<HTMLTableRowElement>(fixture, '[data-testid="umdh-growth"] tbody tr');
    rows[1]!.click();
    await settle(fixture, 6);
    expect(view.stackKey()).toBe('retry');
    const base = el<HTMLSelectElement>(fixture, '[data-testid="umdh-base"]');
    base.value = '';
    base.dispatchEvent(new Event('change'));
    await settle(fixture, 6);
    expect(store.umdhBase()).toBeNull();
    const target = el<HTMLSelectElement>(fixture, '[data-testid="umdh-target"]');
    target.value = String(store.umdh()[1]!.id);
    target.dispatchEvent(new Event('change'));
    await settle(fixture, 6);
    expect(store.umdhTarget()).toBe(store.umdh()[1]!.id);
    base.value = String(store.umdh()[0]!.id);
    base.dispatchEvent(new Event('change'));
    await settle(fixture, 6);
    expect(store.umdhBase()).toBe(store.umdh()[0]!.id);
  });

  it('expands the stack tree lazily', async () => {
    const { fixture, view } = await mount();
    tab(fixture, 'growth').click();
    tab(fixture, 'tree').click();
    await settle(fixture, 8);
    expect(view.tree()).toHaveLength(1);
    const root = view.tree()[0]!;
    await view.expanded(root, true);
    await settle(fixture, 6);
    expect(view.tree().length).toBeGreaterThan(1);
    const before = view.tree().length;
    await view.expanded(root, true);
    expect(view.tree()).toHaveLength(before);
    await view.expanded(root, false);
    expect(el(fixture, '[data-testid="umdh-tree"]').textContent).toContain('OrderService!wmain');
    const toggle = el<HTMLButtonElement>(fixture, '[data-testid="umdh-tree"] button');
    expect(toggle.getAttribute('aria-label')).toMatch(/^(Expand|Collapse) OrderService!wmain$/);
  });

  it('shows tree failures, explains and closes', async () => {
    const backend = new MockDumpsBackend();
    backend.umdhTree = () => Promise.reject(new CmdError({ code: 'invalid', message: 'tree failed', retryable: true }));
    const { fixture, explain, store, view } = await mount(true, backend);
    tab(fixture, 'tree').click();
    await settle(fixture, 6);
    expect(text(fixture)).toContain('tree failed');
    await view.expanded({ node: node('a'), path: ['a'], level: 0 }, true);
    expect(view.treeError()).toBe('tree failed');
    tab(fixture, 'findings').click();
    await settle(fixture, 6);
    el<HTMLButtonElement>(fixture, '[data-testid="dumps-explain"]').click();
    await settle(fixture, 4);
    expect(explain).toHaveBeenCalledWith(store.umdh().map((d) => d.id));
    explain.mockRejectedValueOnce(new Error('no model'));
    el<HTMLButtonElement>(fixture, '[data-testid="dumps-explain"]').click();
    await settle(fixture, 4);
    expect(text(fixture)).toContain('no model');
    for (let i = 0; i < 3; i++) {
      buttonByText(fixture, 'Close').click();
      await settle(fixture, 6);
    }
    expect(text(fixture)).toContain('Open UMDH snapshots');
  });
});
