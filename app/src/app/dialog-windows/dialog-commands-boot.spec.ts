import { TestBed } from '@angular/core/testing';
import { settle } from '../../testing/helpers';
import { MockAiBackend } from '../core/mock/mock-ai-backend';
import { dialogAppConfig } from './dialog-bootstrap';
import { DIALOG_BACKEND_COMMANDS, DIALOG_BASE_COMMANDS, DIALOG_BOOT_COMMANDS, DIALOG_SYNC_COMMANDS } from './dialog-def';
import { DIALOG_IO } from './dialog-io';
import { FakeDialogIo } from './dialog-io-fake';
import { DIALOG_TEST_INPUTS } from './dialog-test-inputs';
import { DIALOG_DEFS } from './dialog-registry';
import { DIALOG_ROUTE } from './dialog-route';
import { DialogShell } from './dialog-shell';
import { commandCovered, KIT_BASE_COMMANDS } from './kit';

/**
 * The contract enforced where Tauri enforces it: every dialog boots with its real Tauri backends over one invoke seam
 * (`window.__TAURI_INTERNALS__.invoke`, which Tauri's own `invoke` calls), and every command it sends must be one its
 * declared list or the base set names. The fake answers nothing useful (null), so this checks which commands a dialog
 * sends while it starts and what Rust would refuse; the lint covers what a click would send.
 */
/** An answer that tolerates any use: every property is itself, it iterates as empty, has length 0 and prints as ''. */
const ANY: unknown = new Proxy(function () {}, {
  get: (_t, k) => (k === Symbol.iterator ? function* () {} : k === 'then' ? undefined : k === 'length' || k === 'size' ? 0 : k === Symbol.toPrimitive ? () => '' : ANY),
  apply: () => ANY,
});

/**
 * Answers that state reducers copy (a spread of the placeholder keeps none of its fields), taken from the mock backend so
 * they have the real shape.
 */
const ANSWERS: Record<string, unknown> = {};

describe('every dialog sends only the commands it declares', () => {
  const sent: string[] = [];
  const w = window as unknown as { __TAURI_INTERNALS__?: unknown };
  /**
   * The test whose window is booting. A call made while no test is live (work a torn-down window had left in flight)
   * is not counted and never answered, so it cannot go on to send more in the next test's name.
   */
  let live = false;

  beforeAll(async () => {
    // The AI backend is loaded lazily on its first call. Loaded here, a window's first AI call (the status store's
    // refresh) goes out while that window's test runs, not when the chunk happens to arrive (in a later test).
    await import('../ai/ai-backend-tauri');
    ANSWERS['ai_setup_plan'] = await new MockAiBackend({ mode: 'setup' }).setupPlan();
  });

  beforeEach(() => {
    sent.length = 0;
    live = true;
    w.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'd-main-1' }, currentWebview: { windowLabel: 'd-main-1', label: 'd-main-1' } },
      transformCallback: () => 1,
      unregisterCallback: () => undefined,
      convertFileSrc: (p: string) => p,
      invoke: (cmd: string) => {
        if (!live) return new Promise(() => undefined);
        if (!cmd.startsWith('plugin:')) sent.push(cmd);
        return Promise.resolve(cmd in ANSWERS ? ANSWERS[cmd] : ANY);
      },
    };
    (window as unknown as { __TAURI_EVENT_PLUGIN_INTERNALS__?: unknown }).__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => undefined };
  });
  afterEach(() => {
    live = false;
    // Tearing the module down unlistens through the event internals, so they go last. Stale calls keep finding an
    // invoke (which no longer answers) instead of throwing into whichever test runs next.
    TestBed.resetTestingModule();
  });
  afterAll(() => {
    delete w.__TAURI_INTERNALS__;
    delete (window as unknown as { __TAURI_EVENT_PLUGIN_INTERNALS__?: unknown }).__TAURI_EVENT_PLUGIN_INTERNALS__;
  });

  for (const def of DIALOG_DEFS) {
    it(`${def.id}`, async () => {
      const spy = vi.spyOn(console, 'error').mockImplementation(() => undefined);
      try {
        const io = new FakeDialogIo();
        io.start = { dialogId: def.id, title: def.id, modality: def.modality, input: DIALOG_TEST_INPUTS[def.id] ?? {} };
        const config = await dialogAppConfig(def.id);
        // The component's chunk is loaded before the window boots, so its whole start runs inside this test.
        await def.load();
        TestBed.configureTestingModule({
          providers: [...config.providers, { provide: DIALOG_IO, useValue: io }, { provide: DIALOG_ROUTE, useValue: { id: def.id, host: 'main', label: 'd-main-1' } }],
        });
        const fixture = TestBed.createComponent(DialogShell);
        await settle(fixture, 8);
        const allowed = [...KIT_BASE_COMMANDS, ...DIALOG_BASE_COMMANDS, ...DIALOG_BOOT_COMMANDS, ...DIALOG_SYNC_COMMANDS, ...def.commands, ...(def.backends ?? []).flatMap((b) => DIALOG_BACKEND_COMMANDS[b] ?? [])];
        const refused = [...new Set(sent)].filter((c) => !commandCovered(allowed, c));
        expect(refused, `${def.id} sent commands its contract does not declare`).toEqual([]);
        fixture.destroy();
      } finally {
        spy.mockRestore();
      }
    });
  }
});
