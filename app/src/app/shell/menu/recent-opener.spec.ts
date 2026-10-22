import { TestBed } from '@angular/core/testing';
import { provideTinyBackend, settle } from '../../../testing/helpers';
import { provideToolsBackend } from '../../core/tools-backend';
import { RecentService } from '../../core/recent/recent.service';
import { WorkspaceStore } from '../../core/workspace.store';
import { ProjectService } from '../../projects/project.service';
import { UnsavedChangesGuard } from '../../projects/unsaved-changes';
import { RecentOpener } from './recent-opener';

function setup() {
  TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideToolsBackend()] });
  return TestBed.inject(RecentOpener);
}

describe('RecentOpener', () => {
  it('opens a recent file into the workspace', async () => {
    const opener = setup();
    const open = vi.spyOn(TestBed.inject(WorkspaceStore), 'openPaths').mockResolvedValue(undefined as never);
    await opener.openFile('/logs/a.log');
    expect(open).toHaveBeenCalledWith(['/logs/a.log']);
  });

  it('opens a recent project only when the unsaved-changes question is answered yes', async () => {
    const opener = setup();
    const guard = TestBed.inject(UnsavedChangesGuard);
    const open = vi.spyOn(TestBed.inject(ProjectService), 'open').mockResolvedValue(true);
    vi.spyOn(guard, 'confirm').mockResolvedValueOnce(false).mockResolvedValueOnce(true);
    await opener.openProject('/p/x.imtproj');
    expect(open).not.toHaveBeenCalled();
    await opener.openProject('/p/x.imtproj');
    expect(open).toHaveBeenCalledWith('/p/x.imtproj');
  });

  it('empties both recent lists', async () => {
    const opener = setup();
    const recent = TestBed.inject(RecentService);
    const clear = vi.spyOn(recent, 'clear').mockResolvedValue();
    await opener.clear();
    await settle();
    expect(clear.mock.calls.map((c) => c[0]).sort()).toEqual(['files', 'projects']);
  });
});
