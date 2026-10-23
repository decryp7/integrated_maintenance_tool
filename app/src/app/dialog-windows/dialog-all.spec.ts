import { TestBed } from '@angular/core/testing';
import { settle } from '../../testing/helpers';
import { dialogAppConfig } from './dialog-bootstrap';
import { DIALOG_IO } from './dialog-io';
import { FakeDialogIo } from './dialog-io-fake';
import { DIALOG_TEST_INPUTS } from './dialog-test-inputs';
import { DIALOG_DEFS } from './dialog-registry';
import { DIALOG_ROUTE } from './dialog-route';
import { DialogShell } from './dialog-shell';

/**
 * Every registered dialog boots through the real dialog bootstrap (`dialogAppConfig`: the base providers plus the
 * dialog's feature groups) and renders its content in the real shell, with fixture inputs for the ones that need them.
 * A missing provider (NG0201/NG0200) or any logged error fails here, not in a user's blank window.
 */
describe('every dialog window', () => {
  for (const def of DIALOG_DEFS) {
    it(`${def.id} opens with only its dialog providers`, async () => {
      const errors: string[] = [];
      const spy = vi.spyOn(console, 'error').mockImplementation((...a: unknown[]) => void errors.push(a.map((x) => (typeof x === 'object' ? JSON.stringify(x) : String(x))).join(' ')));
      try {
        const io = new FakeDialogIo();
        io.start = { dialogId: def.id, title: def.id, modality: def.modality, input: DIALOG_TEST_INPUTS[def.id] ?? {} };
        const config = await dialogAppConfig(def.id);
        TestBed.configureTestingModule({
          providers: [
            ...config.providers,
            { provide: DIALOG_IO, useValue: io },
            { provide: DIALOG_ROUTE, useValue: { id: def.id, host: 'main', label: 'd-main-1' } },
          ],
        });
        const fixture = TestBed.createComponent(DialogShell);
        await settle(fixture, 1);
        const loading = () => (fixture.nativeElement as HTMLElement).querySelector('[data-testid=dialog-loading]');
        for (let i = 0; i < 400 && loading(); i++) await settle(fixture, 1);
        if (loading()) throw new Error(`${def.id} never left Loading: ${errors.join(' | ')}`);
        await settle(fixture, 4);
        const root = fixture.nativeElement as HTMLElement;
        expect(errors.filter((e) => /NG0\d+|No provider|NullInjector/.test(e))).toEqual([]);
        expect(root.querySelector('[data-testid=dialog-error]')?.textContent ?? '').toBe('');
        expect(root.querySelector('[data-testid=dialog-loading]')).toBeNull();
        expect(root.querySelector('main')!.children.length).toBeGreaterThan(0);
        expect(errors.filter((e) => /NG0\d+|No provider|NullInjector/.test(e))).toEqual([]);
        // The mock engine refuses to preview files outside the (empty) workspace; that is the mock's rule, not a missing provider.
        expect(errors.filter((e) => !/readFileHead/.test(e))).toEqual([]);
        fixture.destroy();
      } finally {
        spy.mockRestore();
        TestBed.resetTestingModule();
      }
    });
  }
});
