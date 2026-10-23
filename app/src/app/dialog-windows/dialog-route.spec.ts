import { dialogHash, parseDialogRoute } from './dialog-route';

describe('dialog route', () => {
  it('round-trips and refuses anything malformed', () => {
    const route = { id: 'ai-setup', host: 'main', label: 'd-main-1' };
    expect(parseDialogRoute(dialogHash(route))).toEqual(route);
    expect(parseDialogRoute('#/dialog?id=settings&host=w-2&label=d-w-2-3')).toEqual({ id: 'settings', host: 'w-2', label: 'd-w-2-3' });
    for (const bad of ['', '#/popout?view=a&host=main&label=p-main-1', '#/dialog?id=&host=main&label=d-main-1', '#/dialog?id=../x&host=main&label=d-main-1', '#/dialog?id=a&host=main&label=p-main-1', '#/dialog?id=a&host=&label=d-main-1', '#/dialog', '#/dialog?id=a&host=ma in&label=d-main-1']) {
      expect(parseDialogRoute(bad)).toBeNull();
    }
    expect(parseDialogRoute(undefined as unknown as string)).toBeNull();
  });
});
