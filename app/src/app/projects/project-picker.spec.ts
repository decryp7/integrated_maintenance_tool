import { TestBed } from '@angular/core/testing';
import { ProjectPicker } from './project-picker';

const dialog = vi.hoisted(() => ({ open: vi.fn(), save: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => dialog);

describe('ProjectPicker', () => {
  afterEach(() => {
    delete (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__'];
    dialog.open.mockReset();
    dialog.save.mockReset();
  });

  it('answers with demo paths in a plain browser and offers the last saved one again', async () => {
    const picker = TestBed.inject(ProjectPicker);
    expect(await picker.pickOpen()).toBe('/demo/projects/untitled.imtproj');
    expect(await picker.pickSave('Case 1.imtproj')).toBe('/demo/projects/Case 1.imtproj');
    expect(await picker.pickOpen()).toBe('/demo/projects/Case 1.imtproj');
    expect(dialog.open).not.toHaveBeenCalled();
  });

  it('uses the native dialogs inside Tauri', async () => {
    (window as unknown as Record<string, unknown>)['__TAURI_INTERNALS__'] = {};
    const picker = TestBed.inject(ProjectPicker);
    dialog.open.mockResolvedValueOnce('/p/a.imtproj').mockResolvedValueOnce(null).mockResolvedValueOnce(['x']);
    expect(await picker.pickOpen()).toBe('/p/a.imtproj');
    expect(await picker.pickOpen()).toBeNull();
    expect(await picker.pickOpen()).toBeNull();
    expect(dialog.open.mock.calls[0][0]).toMatchObject({ multiple: false, filters: [{ extensions: ['imtproj'] }] });
    dialog.save.mockResolvedValue('/p/b.imtproj');
    expect(await picker.pickSave('b.imtproj')).toBe('/p/b.imtproj');
    expect(dialog.save.mock.calls[0][0]).toMatchObject({ defaultPath: 'b.imtproj' });
  });
});
