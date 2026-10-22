import { TestBed } from '@angular/core/testing';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DebriefBackend } from '../core/backend';
import { WorkspaceStore } from '../core/workspace.store';
import { UndoStack } from '../undo/undo-stack';
import { provideTinyBackend, settle } from '../../testing/helpers';
import { ProfileEdits } from './profile-edits';

describe('ProfileEdits', () => {
  afterEach(() => TestBed.resetTestingModule());

  function setup() {
    TestBed.configureTestingModule({ providers: [provideTinyBackend()] });
    return { edits: TestBed.inject(ProfileEdits), store: TestBed.inject(WorkspaceStore), backend: TestBed.inject(DebriefBackend), undo: TestBed.inject(UndoStack) };
  }

  it('apply saves the profile and returns the re-import job at once; the workspace follows it in the background', async () => {
    const { edits, store, backend, undo } = setup();
    await settle();
    const version = store.profileVersion();
    const job = await edits.apply((m) => {
      m.name = 'Edited';
    }, 'drop step');
    expect(job).toEqual(expect.any(Number));
    // The profile is saved already; the status bar shows the re-import still running.
    expect((await backend.getProfile()).name).toBe('Edited');
    expect(store.busy()).toEqual({ kind: 'import', label: 'Applying the profile…', jobId: job });
    expect(undo.undoLabel()).toBe('drop step');
    // The summary is refreshed after the job ends: wait for both (a loaded CI machine stretches the gap).
    await vi.waitFor(
      () => {
        expect(store.busy()).toBeNull();
        expect(store.summary()?.profileName).toBe('Edited');
      },
      { timeout: 20_000 },
    );
    expect(store.profileVersion()).toBe(version + 1);
  });

  it('apply refuses an invalid profile before saving or starting a job', async () => {
    const { edits, store, backend } = setup();
    await settle();
    const save = vi.spyOn(backend, 'saveProfile');
    vi.spyOn(backend, 'validateProfile').mockResolvedValueOnce({ ok: false, error: 'unknown key', line: 2 });
    await expect(edits.apply(() => undefined)).rejects.toThrow('unknown key');
    vi.spyOn(backend, 'validateProfile').mockResolvedValueOnce({ ok: false, error: null, line: null });
    await expect(edits.apply(() => undefined)).rejects.toThrow('The profile is not valid.');
    expect(save).not.toHaveBeenCalled();
    expect(store.busy()).toBeNull();
  });
});
