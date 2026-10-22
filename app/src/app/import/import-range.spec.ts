import { TestBed } from '@angular/core/testing';
import { settle } from '../../testing/helpers';
import { ImtBackend } from '../core/backend';
import { MockBackend } from '../core/mock/mock-backend';
import { Platform } from '../core/platform';
import { WorkspaceStore } from '../core/workspace.store';
import { DialogService } from '../ui/dialog.service';
import { ImportRange, parseRangeTime, rangeProblem } from './import-range';
import { ImportRangeDialog } from './import-range-dialog';

describe('time range helpers', () => {
  it('reads datetime-local text as UTC and refuses other text', () => {
    expect(parseRangeTime('1970-01-01T00:01')).toBe(60_000);
    expect(parseRangeTime('1970-01-01T01:00:00+01:00')).toBe(0);
    expect(parseRangeTime(' ')).toBeNull();
    expect(parseRangeTime('soon')).toBeNull();
  });

  it('explains what is wrong with a range', () => {
    expect(rangeProblem(null, null, '', '')).toContain('start, an end');
    expect(rangeProblem(null, null, 'x', '')).toContain('start is not');
    expect(rangeProblem(1, null, '1', 'y')).toContain('end is not');
    expect(rangeProblem(5, 1, 'a', 'b')).toContain('before');
    expect(rangeProblem(1, 5, 'a', 'b')).toBeNull();
    expect(rangeProblem(1, null, 'a', '')).toBeNull();
  });
});

describe('ImportRange', () => {
  function setup(pick: string[]) {
    const backend = new MockBackend({ latency: false });
    TestBed.configureTestingModule({
      providers: [
        { provide: ImtBackend, useValue: backend },
        { provide: Platform, useValue: { pickPaths: async () => pick, isTauri: () => false } },
      ],
    });
    return { backend, dialogs: TestBed.inject(DialogService), range: TestBed.inject(ImportRange), store: TestBed.inject(WorkspaceStore) };
  }

  it('opens the picked paths with the chosen range and clears the range afterwards', async () => {
    const { backend, dialogs, range, store } = setup(['/demo/range-folder']);
    const windows: unknown[] = [];
    const set = backend.setImportWindow.bind(backend);
    backend.setImportWindow = (f, t) => {
      windows.push([f, t]);
      return set(f, t);
    };
    const done = range.pickAndOpen(true);
    await settle(undefined, 2);
    const open = dialogs.stack().at(-1);
    expect(open?.component).toBe(ImportRangeDialog);
    open?.ref.close({ from: 1000, to: 2000 });
    await done;
    await settle(undefined, 4);
    expect(windows).toEqual([[1000, 2000], [null, null]]);
    expect(backend.importWindow).toBeNull();
    expect(store.summary()?.sources.some((s) => s.path === '/demo/range-folder')).toBe(true);
  });

  it('does nothing when the picker or the dialog is cancelled', async () => {
    const none = setup([]);
    await none.range.pickAndOpen(true);
    expect(none.dialogs.stack()).toHaveLength(0);
    TestBed.resetTestingModule();
    const { dialogs, range, backend } = setup(['/demo/x']);
    const done = range.pickAndOpen(false);
    await settle(undefined, 2);
    dialogs.stack().at(-1)?.ref.close();
    await done;
    expect(backend.importWindow).toBeNull();
  });

  it('shows an engine refusal of the range as the workspace error and opens nothing', async () => {
    const { dialogs, range, store, backend } = setup(['/demo/y']);
    const done = range.pickAndOpen(true);
    await settle(undefined, 2);
    dialogs.stack().at(-1)?.ref.close({ from: 5, to: 1 });
    await done;
    expect(store.error()).toContain('before');
    expect(store.summary()?.sources.some((s) => s.path === '/demo/y')).toBe(false);
    expect(backend.importWindow).toBeNull();
  });
});
