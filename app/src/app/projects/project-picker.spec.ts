import { TestBed } from '@angular/core/testing';
import { ProjectPicker } from './project-picker';

// The engine shows the dialogs (`pick_paths`) and answers with the chosen paths. Tauri's IPC is stubbed at its
// window internals: the shared test setup has already loaded `@tauri-apps/api/core` (through the mocks).
const core = { invoke: vi.fn() };

describe('ProjectPicker', () => {
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__'];
    core.invoke.mockReset();
  });

  it('answers with demo paths in a plain browser and offers the last saved one again', async () => {
    const picker = TestBed.inject(ProjectPicker);
    expect(await picker.pickOpen()).toBe('/demo/projects/untitled.imtproj');
    expect(await picker.pickSave('Case 1.imtproj')).toBe('/demo/projects/Case 1.imtproj');
    expect(await picker.pickOpen()).toBe('/demo/projects/Case 1.imtproj');
    expect(core.invoke).not.toHaveBeenCalled();
  });

  it('uses the native dialogs inside Tauri', async () => {
    (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__'] = { invoke: core.invoke };
    const picker = TestBed.inject(ProjectPicker);
    core.invoke.mockResolvedValueOnce(['/p/a.imtproj']).mockResolvedValueOnce([]);
    expect(await picker.pickOpen()).toBe('/p/a.imtproj');
    expect(await picker.pickOpen()).toBeNull();
    expect(core.invoke.mock.calls[0]?.[0]).toBe('pick_paths');
    expect(core.invoke.mock.calls[0]?.[1]).toEqual({ request: expect.objectContaining({ kind: 'openFile', filters: [expect.objectContaining({ extensions: ['imtproj'] })] }) });
    core.invoke.mockResolvedValue(['/p/b.imtproj']);
    expect(await picker.pickSave('b.imtproj')).toBe('/p/b.imtproj');
    expect(core.invoke.mock.calls.at(-1)?.[1]).toMatchObject({ request: { kind: 'save', defaultName: 'b.imtproj' } });
  });
});
