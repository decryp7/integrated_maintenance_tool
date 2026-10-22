import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, provideTinyBackend, settle, text, tinyBackend } from '../../../testing/helpers';
import { WorkspaceStore } from '../../core/workspace.store';
import { FormatWizard } from '../../format/format-wizard';
import { DialogService } from '../../ui/dialog.service';
import { EngineSettingsStore } from '../engine-settings.store';
import { SettingsScreen } from '../settings-screen';
import { MockParsersBackend } from './mock-parsers-backend';
import { ParserEditor } from './parser-editor';
import { DEMO_PARSER_IMPORT, ParserFiles } from './parser-files';
import { ParsersBackend } from './parsers-backend';
import { ParsersSettings } from './parsers-settings';
import { PARSERS_ID } from './parsers.types';

async function setup() {
  const imt = tinyBackend();
  const backend = new MockParsersBackend(imt);
  const files = { pickImport: vi.fn().mockResolvedValue(DEMO_PARSER_IMPORT), pickExport: vi.fn().mockResolvedValue('/demo/exports/nlog.toml') };
  TestBed.configureTestingModule({ providers: [provideTinyBackend(imt), { provide: ParsersBackend, useValue: backend }, { provide: ParserFiles, useValue: files }] });
  await TestBed.inject(WorkspaceStore).init();
  const fixture = TestBed.createComponent(ParsersSettings);
  await settle(fixture);
  await vi.waitFor(() => expect(all(fixture, 'li.parser').length).toBeGreaterThan(0));
  await settle(fixture);
  return { fixture, backend, files, dialogs: TestBed.inject(DialogService) };
}

const row = (fixture: Parameters<typeof el>[0], id: string) => el(fixture, `li[data-parser="${id}"]`);

describe('ParsersSettings', () => {
  it('lists the parsers by category with version, origin, kind and the files they read', async () => {
    const { fixture } = await setup();
    expect(all(fixture, 'h4').map((h) => h.textContent?.trim())).toEqual(['.NET', 'Big data', 'Generic', 'Mobile', 'System', 'Web servers', 'Parser files that could not be read']);
    const clef = row(fixture, 'serilog-clef');
    expect(clef.textContent).toContain('Serilog CLEF');
    expect(clef.textContent).toContain('1.0.0');
    expect(clef.textContent).toContain('Built-in');
    expect(clef.textContent).toContain('json');
    expect(el(fixture, '[data-testid=parser-read-serilog-clef]').textContent).toBe('Read 1 file: app.clef');
    expect(el(fixture, '[data-testid=parser-read-nlog]').textContent).toBe('Read no files in this window');
    // Built-ins cannot be removed, only disabled.
    expect(clef.querySelector('[aria-label="Remove Serilog CLEF"]')).toBeNull();
    expect(clef.querySelector('[aria-label="Edit Serilog CLEF"]')).not.toBeNull();
    expect(text(fixture)).toContain('legacy-gateway.toml');
    expect(text(fixture)).toContain('Line 1, column 1: missing field `id`');
  });

  it('enables and disables a parser with its switch', async () => {
    const { fixture } = await setup();
    const sw = row(fixture, 'nlog').querySelector<HTMLInputElement>('input[role=switch]')!;
    expect(sw.checked).toBe(true);
    sw.click();
    await vi.waitFor(() => expect(row(fixture, 'nlog').classList).toContain('off'));
    await settle(fixture);
    expect(row(fixture, 'nlog').querySelector<HTMLInputElement>('input[role=switch]')!.checked).toBe(false);
  });

  it('imports a parser file, shows its origin and offers to re-read the sources it would now read', async () => {
    const { fixture, backend } = await setup();
    // A parser that reads app.log better than the built-in that read it.
    await backend.save('schema = 1\nid = "web-log"\nname = "Web log"\nversion = "1.0.0"\ncategory = "Custom"\n[match]\nfiles = ["app.log"]\npriority = 5\n[format]\nkind = "text"\n');
    buttonByText(fixture, 'Import .toml…').click();
    await vi.waitFor(() => expect(text(fixture)).toContain('Imported Inventory sync CSV 1.0.0.'));
    await settle(fixture);
    expect(row(fixture, 'inventory-csv').textContent).toContain('Custom');
    expect(el(fixture, '[data-testid=parsers-stale]').textContent).toContain('1 source was read with a parser that changed.');
    buttonByText(fixture, 'Re-read').click();
    await vi.waitFor(() => expect(text(fixture)).toContain('Read the files again with their parsers.'), { timeout: 10_000 });
    await settle(fixture);
    expect(all(fixture, '[data-testid=parsers-stale]')).toHaveLength(0);
    expect(el(fixture, '[data-testid=parser-read-web-log]').textContent).toBe('Read 1 file: app.log');
    buttonByText(fixture, 'Dismiss').click();
    await settle(fixture);
    expect(all(fixture, '[data-testid=parsers-notice]')).toHaveLength(0);
  });

  it('opens the editor, removes user parsers after a confirmation and exports', async () => {
    const { fixture, backend, dialogs } = await setup();
    el(fixture, '[aria-label="Edit NLog default layout"]').click();
    await vi.waitFor(() => expect(dialogs.stack().at(-1)?.component).toBe(ParserEditor));
    expect(dialogs.stack().at(-1)?.options.title).toBe('Edit parser nlog');
    dialogs.stack().at(-1)!.ref.close();
    await settle(fixture);

    el(fixture, '[aria-label="Duplicate NLog default layout"]').click();
    await vi.waitFor(() => expect(text(fixture)).toContain('Saved a copy as nlog-copy.'));
    await settle(fixture);
    const copy = row(fixture, 'nlog-copy');
    expect(copy.textContent).toContain('Custom');
    copy.querySelector<HTMLButtonElement>('[aria-label="Remove NLog default layout (copy)"]')!.click();
    await vi.waitFor(() => expect(dialogs.stack().at(-1)?.options.title).toBe('Remove parser?'));
    dialogs.stack().at(-1)!.ref.close(true);
    await vi.waitFor(() => expect(all(fixture, 'li[data-parser="nlog-copy"]')).toHaveLength(0));

    const exportTo = vi.spyOn(backend, 'exportTo');
    el(fixture, '[aria-label="Export… NLog default layout"]').click();
    await vi.waitFor(() => expect(exportTo).toHaveBeenCalledWith('nlog', '/demo/exports/nlog.toml', false));
    await vi.waitFor(() => expect(text(fixture)).toContain('Exported nlog to /demo/exports/nlog.toml.'));
  });

  it('starts the editor blank and the format wizard from a file of the window', async () => {
    const { fixture, dialogs } = await setup();
    buttonByText(fixture, 'New blank').click();
    await vi.waitFor(() => expect(dialogs.stack().at(-1)?.options.title).toBe('New parser'));
    dialogs.stack().at(-1)!.ref.close();
    await settle(fixture);
    buttonByText(fixture, 'New from a file…').click();
    await settle(fixture);
    const item = [...document.querySelectorAll<HTMLButtonElement>('[cdkmenuitem], .mi')].find((b) => b.textContent?.includes('app.log'));
    expect(item).toBeTruthy();
    item!.click();
    await vi.waitFor(() => expect(dialogs.stack().at(-1)?.component).toBe(FormatWizard), { timeout: 10_000 });
    expect(dialogs.stack().at(-1)?.options).toMatchObject({ title: 'New parser from app.log', inputs: { path: '/logs/web02/app.log', name: 'app.log' } });
  });

  it('shows a failed listing with Retry', async () => {
    const { fixture, backend } = await setup();
    const list = vi.spyOn(backend, 'list').mockRejectedValueOnce(new Error('engine gone'));
    fixture.componentInstance.ngOnInit();
    await vi.waitFor(() => expect(text(fixture)).toContain('Could not list the parsers: engine gone'));
    buttonByText(fixture, 'Retry').click();
    await vi.waitFor(() => expect(list).toHaveBeenCalledTimes(2));
    await settle(fixture);
    expect(text(fixture)).not.toContain('engine gone');
  });

  it('is a section of the Settings screen', async () => {
    await setup();
    TestBed.inject(EngineSettingsStore).select(PARSERS_ID);
    const screen = TestBed.createComponent(SettingsScreen);
    await settle(screen);
    expect(el(screen, '[data-testid=settings-nav-parsers]').getAttribute('aria-current')).toBe('page');
    expect(all(screen, 'imt-parsers-settings')).toHaveLength(1);
  });
});
