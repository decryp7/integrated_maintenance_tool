import { TestBed } from '@angular/core/testing';
import { provideTinyBackend } from '../../testing/helpers';
import { POPOUT_ROUTE } from '../popout/popout-route';
import { startWindowing } from './provide-windowing';
import { WindowTitleSync } from './window-title';
import { WindowingService } from './windowing.service';

describe('startWindowing', () => {
  function start(route: unknown) {
    TestBed.configureTestingModule({ providers: [...provideTinyBackend(), { provide: POPOUT_ROUTE, useValue: route }] });
    const init = vi.spyOn(TestBed.inject(WindowingService), 'init').mockResolvedValue();
    const title = vi.spyOn(TestBed.inject(WindowTitleSync), 'start').mockImplementation(() => undefined);
    TestBed.runInInjectionContext(() => startWindowing());
    return { init, title };
  }

  it('starts the window events and title sync in an app window', () => {
    const { init, title } = start(null);
    expect(init).toHaveBeenCalled();
    expect(title).toHaveBeenCalled();
  });

  it('does neither in a pop-out panel window (its host owns them)', () => {
    const { init, title } = start({ view: 'inspector', host: 'main', label: 'p-main-1' });
    expect(init).not.toHaveBeenCalled();
    expect(title).not.toHaveBeenCalled();
  });
});
