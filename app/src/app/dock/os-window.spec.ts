import { TestBed } from '@angular/core/testing';
import { BrowserOsWindow, FULL_SCREEN_MODE_KEY, FullScreenPrefs, loadFullScreenMode, OsWindow } from './os-window';

describe('full screen', () => {
  beforeEach(() => localStorage.removeItem(FULL_SCREEN_MODE_KEY));

  it('remembers the mode, "both" by default or when the saved value is unknown or unreadable', () => {
    expect(loadFullScreenMode()).toBe('both');
    TestBed.inject(FullScreenPrefs).set('os');
    expect(loadFullScreenMode()).toBe('os');
    localStorage.setItem(FULL_SCREEN_MODE_KEY, 'weird');
    expect(loadFullScreenMode()).toBe('both');
    const spies = [
      vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => { throw new Error('blocked'); }),
      vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => { throw new Error('blocked'); }),
    ];
    expect(loadFullScreenMode()).toBe('both');
    expect(() => TestBed.inject(FullScreenPrefs).set('chrome')).not.toThrow();
    spies.forEach((s) => s.mockRestore());
  });

  it('uses the Fullscreen API in a browser and swallows refusals', async () => {
    expect(TestBed.inject(OsWindow)).toBeInstanceOf(BrowserOsWindow);
    let element: Element | null = null;
    const doc = {
      get fullscreenElement() {
        return element;
      },
      documentElement: { requestFullscreen: vi.fn(async () => void (element = {} as Element)) },
      exitFullscreen: vi.fn(async () => void (element = null)),
    } as unknown as Document;
    const w = new BrowserOsWindow(doc);
    await w.setFullscreen(true);
    await w.setFullscreen(true);
    expect(doc.documentElement.requestFullscreen).toHaveBeenCalledTimes(1);
    await w.setFullscreen(false);
    await w.setFullscreen(false);
    expect(doc.exitFullscreen).toHaveBeenCalledTimes(1);
    (doc.documentElement.requestFullscreen as ReturnType<typeof vi.fn>).mockRejectedValue(new Error('no gesture'));
    await expect(w.setFullscreen(true)).resolves.toBeUndefined();
  });
});
