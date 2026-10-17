import { TestBed } from '@angular/core/testing';
import { buttonByText, el, settle } from '../../testing/helpers';
import { FILE_EXISTS_SUFFIX, fileExistsPath } from '../core/api.types';
import { Platform } from '../core/platform';
import { DialogHost } from './dialog-host';
import { DialogService } from './dialog.service';
import { OverwriteGuard } from './overwrite-guard';

describe('OverwriteGuard', () => {
  const exists = (path: string) => new Error(`${path}${FILE_EXISTS_SUFFIX}`);

  function setup(nativeConfirms = false) {
    TestBed.configureTestingModule({ providers: [{ provide: Platform, useValue: { saveDialogConfirmsOverwrite: () => nativeConfirms } }] });
    const fixture = TestBed.createComponent(DialogHost);
    return { fixture, guard: TestBed.inject(OverwriteGuard), dialogs: TestBed.inject(DialogService) };
  }

  it('recognises the engine error and nothing else', () => {
    expect(fileExistsPath(`/tmp/a.csv${FILE_EXISTS_SUFFIX}`)).toBe('/tmp/a.csv');
    expect(fileExistsPath(exists('/x'))).toBe('/x');
    expect(fileExistsPath('disk full')).toBeNull();
    expect(fileExistsPath(42)).toBeNull();
  });

  it('writes without permission first and returns the result when the file is new', async () => {
    const { guard } = setup();
    const write = vi.fn(async (overwrite: boolean) => `wrote:${overwrite}`);
    expect(await guard.write('/out/a.csv', write)).toBe('wrote:false');
    expect(write).toHaveBeenCalledTimes(1);
  });

  it('asks before replacing, and writes again with permission after "Replace"', async () => {
    const { guard, fixture } = setup();
    const write = vi.fn(async (overwrite: boolean) => {
      if (!overwrite) throw exists('/out/events.csv');
      return 'replaced';
    });
    const done = guard.write('/out/events.csv', write);
    await settle(fixture);
    expect(el(fixture, '[role=dialog]').textContent).toContain('events.csv already exists');
    expect(el(fixture, '[data-testid=confirm-detail]').textContent).toBe('/out/events.csv');
    buttonByText(fixture, 'Replace').click();
    expect(await done).toBe('replaced');
    expect(write.mock.calls.map((c) => c[0])).toEqual([false, true]);
  });

  it('keeps the existing file when the user declines (button or Escape)', async () => {
    const { guard, fixture, dialogs } = setup();
    const write = vi.fn(async (overwrite: boolean) => {
      if (!overwrite) throw `/out/r.html${FILE_EXISTS_SUFFIX}`;
      return 'replaced';
    });
    let done = guard.write('/out/r.html', write);
    await settle(fixture);
    buttonByText(fixture, 'Keep the existing file').click();
    expect(await done).toBeNull();
    done = guard.write('/out/r.html', write);
    await settle(fixture);
    dialogs.dismissTop();
    expect(await done).toBeNull();
    expect(write).toHaveBeenCalledTimes(2);
  });

  it('passes other failures through untouched', async () => {
    const { guard } = setup();
    await expect(guard.write('/x', async () => Promise.reject(new Error('disk full')))).rejects.toThrow('disk full');
  });

  it('trusts the native save dialog where it already asked, but not for paths the app chose', async () => {
    const { guard } = setup(true);
    const write = vi.fn(async (overwrite: boolean) => overwrite);
    expect(await guard.write('/x', write)).toBe(true);
    expect(await guard.write('/x', write, 'app')).toBe(false);
  });

  it('works with a platform fake that predates the dialog check', async () => {
    TestBed.configureTestingModule({ providers: [{ provide: Platform, useValue: {} }] });
    const write = vi.fn(async (overwrite: boolean) => overwrite);
    expect(await TestBed.inject(OverwriteGuard).write('/x', write)).toBe(false);
  });

  it('falls back to the given path when the error names none', async () => {
    const { guard, fixture } = setup();
    const done = guard.write('/given/name.png', async (o) => {
      if (!o) throw FILE_EXISTS_SUFFIX;
      return 1;
    });
    await settle(fixture);
    expect(el(fixture, '[data-testid=confirm-detail]').textContent).toBe('/given/name.png');
    buttonByText(fixture, 'Replace').click();
    expect(await done).toBe(1);
  });
});
