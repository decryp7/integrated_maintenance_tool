import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, settle, text, tinyBackend, typeInto } from '../../../testing/helpers';
import { DebriefBackend } from '../../core/backend';
import { mockUnsupported } from '../../core/mock/mock-unsupported';
import { RedactionSummary } from '../../core/api.types';
import { WorkspaceStore } from '../../core/workspace.store';
import { ProfileEdits } from '../../profile/profile-edits';
import { provideToolsTesting } from '../../tools/mock/test-support';
import { DIALOG_REF, DialogService } from '../../ui/dialog.service';
import { DEFAULT_DETECTORS, parseFieldList, readRedact, writeRedact } from './redact-section';
import { RedactionBanner } from './redaction-banner';
import { REDACTED_VIEW_KEY, RedactionCommands } from './redaction.commands';
import { RedactionDialog } from './redaction-dialog';
import { RedactionService } from './redaction.service';
import { RedactionSummaryView } from './redaction-summary';
import { provideMockShare } from './share-testing';
import { ShareBackend } from '../share-backend';
import { at } from '../../../testing/must';

const closed = vi.fn();

async function setup() {
  closed.mockClear();
  TestBed.configureTestingModule({
    providers: [...provideToolsTesting(), { provide: DebriefBackend, useValue: tinyBackend() }, provideMockShare(), { provide: DIALOG_REF, useValue: { close: closed, closed: Promise.resolve(undefined) } }],
  });
  const store = TestBed.inject(WorkspaceStore);
  await settle();
  return { store, service: TestBed.inject(RedactionService), backend: TestBed.inject(DebriefBackend) };
}

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

describe('redact section', () => {
  it('reads defaults, values and drops what the engine would reject', () => {
    expect(readRedact({})).toEqual({ detectors: [...DEFAULT_DETECTORS], mode: 'mask', fields: ['@message', '@exception'], onExport: false, rules: [] });
    const form = readRedact({ redact: { detectors: ['guid', 'nope', 'email'], mode: 'hash', fields: ['*'], on_export: true, rule: [{ name: 'n', regex: 'r', replace: 'x' }, { name: 'm', regex: 's' }, null] } });
    expect(form).toEqual({ detectors: ['email', 'guid'], mode: 'hash', fields: ['*'], onExport: true, rules: [{ name: 'n', regex: 'r', replace: 'x' }, { name: 'm', regex: 's', replace: '' }] });
    expect(readRedact({ redact: { mode: 'drop' } }).mode).toBe('drop');
    expect(readRedact({ redact: { mode: 'blur', rule: [{}] } })).toMatchObject({ mode: 'mask', rules: [{ name: '', regex: '', replace: '' }] });
    expect(readRedact({ redact: 'nonsense' } as never).mode).toBe('mask');
  });

  it('writes snake_case keys, rules without an empty replace', () => {
    const model: Record<string, unknown> = {};
    writeRedact(model, { detectors: ['guid', 'email'], mode: 'drop', fields: ['User'], onExport: true, rules: [{ name: 'a', regex: 'b', replace: '' }, { name: 'c', regex: 'd', replace: 'e' }] });
    expect(model['redact']).toEqual({ detectors: ['email', 'guid'], mode: 'drop', fields: ['User'], on_export: true, rule: [{ name: 'a', regex: 'b' }, { name: 'c', regex: 'd', replace: 'e' }] });
    expect(parseFieldList(' A, ,B ,')).toEqual(['A', 'B']);
  });
});

describe('RedactionService and commands', () => {
  it('switches the redacted view in the engine and reloads the grid', async () => {
    const { service, store } = await setup();
    const version = store.dataVersion();
    expect(service.on()).toBe(false);
    expect(await service.set(true)).toBe(true);
    expect(service.on()).toBe(true);
    expect(store.dataVersion()).toBeGreaterThan(version);
    expect(await service.toggle()).toBe(true);
    expect(service.on()).toBe(false);
  });

  it('reports why it cannot switch on', async () => {
    const { service, store, backend } = await setup();
    await backend.saveProfile('name = "x"\n[redact]\ndetectors = []');
    expect(await service.set(true)).toBe(false);
    expect(store.error()).toContain('no redaction rules');
    expect(service.busy()).toBe(false);
  });

  it('says calmly that the redacted view is not available yet with the log store', async () => {
    const { service, store } = await setup();
    vi.spyOn(TestBed.inject(ShareBackend), 'setRedactedView').mockRejectedValueOnce(mockUnsupported('the redacted view is not available here', 'Exports can still be redacted.'));
    expect(await service.set(true)).toBe(false);
    expect(store.error()).toBeNull();
    expect(store.notice()).toBe('The redacted view is not available here. Exports can still be redacted.');
  });

  it('reads the export default from the profile', async () => {
    const { service, backend } = await setup();
    expect(await service.onExportDefault()).toBe(false);
    await backend.saveProfile((await backend.getProfile()).toml.replace('on_export = false', 'on_export = true'));
    expect(await service.onExportDefault()).toBe(true);
    vi.spyOn(TestBed.inject(ProfileEdits), 'current').mockRejectedValueOnce(new Error('x'));
    expect(await service.onExportDefault()).toBe(false);
  });

  it('runs the toggle and settings commands', async () => {
    const { service } = await setup();
    const commands = TestBed.inject(RedactionCommands).commands;
    const toggle = at(commands, 0);
    const settings = at(commands, 1);
    expect(toggle.keybinding).toBe(REDACTED_VIEW_KEY);
    await toggle.run({} as never);
    expect(service.on()).toBe(true);
    void settings.run({} as never);
    await settle();
    await TestBed.inject(DialogService).whenLoaded();
    expect(TestBed.inject(DialogService).stack()[0]?.options.title).toBe('Redaction settings');
  });
});

describe('RedactionDialog', () => {
  async function dialog() {
    const ctx = await setup();
    const fixture = TestBed.createComponent(RedactionDialog);
    await settle(fixture);
    await wait(300);
    await settle(fixture);
    return { ...ctx, fixture };
  }

  const checkbox = (fixture: ReturnType<typeof TestBed.createComponent<RedactionDialog>>, label: string) =>
    all<HTMLLabelElement>(fixture, 'label').find((l) => l.textContent?.includes(label))!.querySelector('input')!;

  it('shows the detectors and a live engine preview of the sample', async () => {
    const { fixture } = await dialog();
    expect(all(fixture, '.grid input[type=checkbox]')).toHaveLength(13);
    expect(el(fixture, '[data-testid=redaction-preview]').textContent).toContain('<email>');
    expect(text(fixture)).toContain('email 1');
    checkbox(fixture, 'e-mail addresses').click();
    await wait(300);
    await settle(fixture);
    expect(el(fixture, '[data-testid=redaction-preview]').textContent).toContain('alice@contoso.com');
    checkbox(fixture, 'e-mail addresses').click();
    typeInto(el<HTMLTextAreaElement>(fixture, 'textarea'), 'mail bob@x.io');
    await wait(300);
    await settle(fixture);
    expect(el(fixture, '[data-testid=redaction-preview]').textContent?.trim()).toBe('mail <email>');
  });

  it('edits the mode, fields and custom rules and saves them to the profile', async () => {
    const { fixture, backend } = await dialog();
    typeInto(el<HTMLSelectElement>(fixture, 'select'), 'hash', 'change');
    checkbox(fixture, '@template').click();
    checkbox(fixture, '@exception').click();
    typeInto(el<HTMLInputElement>(fixture, 'input[placeholder="User, ClientIp"]'), 'User, Ip', 'change');
    checkbox(fixture, 'by default').click();
    buttonByText(fixture, 'Add rule').click();
    buttonByText(fixture, 'Add rule').click();
    await settle(fixture);
    const inputs = all<HTMLInputElement>(fixture, '.rule input');
    typeInto(at(inputs, 0), 'ticket');
    typeInto(at(inputs, 1), 'TCK-(?P<value>\\d+)');
    typeInto(at(inputs, 2), 'TCK-#');
    at(all<HTMLButtonElement>(fixture, '.rule button'), 1).click();
    await wait(300);
    await settle(fixture);
    buttonByText(fixture, 'Save').click();
    await settle(fixture);
    expect(closed).toHaveBeenCalled();
    const toml = (await backend.getProfile()).toml;
    expect(toml).toContain('mode = "hash"');
    expect(toml).toContain('on_export = true');
    expect(toml).toMatch(/fields = \[\s*"@message",\s*"@template",\s*"User",\s*"Ip"\s*\]/);
    expect(toml).toContain('name = "ticket"');
    expect(toml).not.toContain('rule-2');
  });

  it('switches every field on and off', async () => {
    const { fixture } = await dialog();
    checkbox(fixture, 'All fields').click();
    await settle(fixture);
    expect(checkbox(fixture, '@message').disabled).toBe(true);
    checkbox(fixture, 'All fields').click();
    await settle(fixture);
    expect(checkbox(fixture, '@message').checked).toBe(true);
  });

  it('shows engine errors, save failures and load failures', async () => {
    const { fixture } = await dialog();
    buttonByText(fixture, 'Add rule').click();
    await settle(fixture);
    typeInto(at(all<HTMLInputElement>(fixture, '.rule input'), 1), '(');
    await wait(300);
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('invalid regex');
    expect(buttonByText(fixture, 'Save').disabled).toBe(true);
    at(all<HTMLButtonElement>(fixture, '.rule button'), 0).click();
    await wait(300);
    await settle(fixture);
    vi.spyOn(TestBed.inject(ProfileEdits), 'apply').mockRejectedValueOnce(new Error('locked'));
    buttonByText(fixture, 'Save').click();
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('locked');
    buttonByText(fixture, 'Cancel').click();
    expect(closed).toHaveBeenCalled();
  });

  it('reports a profile that cannot be read and a failing preview', async () => {
    await setup();
    vi.spyOn(TestBed.inject(ProfileEdits), 'current').mockRejectedValueOnce(new Error('gone'));
    const fixture = TestBed.createComponent(RedactionDialog);
    await settle(fixture);
    expect(text(fixture)).toContain('Could not read the profile: gone');
  });

  it('shows a preview that failed in the backend', async () => {
    await setup();
    const share = TestBed.inject(ShareBackend);
    vi.spyOn(share, 'previewRedaction').mockRejectedValue(new Error('offline'));
    const fixture = TestBed.createComponent(RedactionDialog);
    await settle(fixture);
    await wait(300);
    await settle(fixture);
    expect(el(fixture, '[role=alert]').textContent).toContain('offline');
  });
});

describe('RedactionDialog closed while a preview is pending', () => {
  // Regression (CI, dialog-commands-boot.spec): the debounced preview still asked the engine after the dialog was
  // destroyed, a call made by a window that is already gone (it landed in the next test's window).
  it('cancels the pending preview when the dialog is destroyed', async () => {
    await setup();
    const preview = vi.spyOn(TestBed.inject(ShareBackend), 'previewRedaction');
    const fixture = TestBed.createComponent(RedactionDialog);
    await settle(fixture);
    preview.mockClear();
    fixture.componentInstance['sample'].set('another sample');
    fixture.detectChanges();
    await settle(fixture);
    fixture.destroy();
    await wait(300);
    expect(preview).not.toHaveBeenCalled();
  });
});

describe('RedactionSummaryView', () => {
  it('shows a placeholder, then the hits and samples', async () => {
    TestBed.configureTestingModule({});
    const fixture = TestBed.createComponent(RedactionSummaryView);
    await settle(fixture);
    expect(text(fixture)).toContain('Checking');
    const s: RedactionSummary = { scanned: 10, changed: 2, truncated: true, hits: [{ rule: 'email', count: 3 }, { rule: 'ipv4', count: 0 }, { rule: 'guid', count: 1 }], samples: [{ recordId: 1, before: 'a@b.io', after: '<email>' }] };
    fixture.componentRef.setInput('summary', s);
    await settle(fixture);
    expect(text(fixture)).toContain('2 of 10+ events change');
    expect(text(fixture)).toContain('email 3,');
    expect(text(fixture)).not.toContain('ipv4');
    expect(el(fixture, '.after').textContent).toBe('<email>');
    fixture.componentRef.setInput('summary', { ...s, truncated: false, hits: [] });
    await settle(fixture);
    expect(text(fixture)).toContain('2 of 10 events change');
  });
});

describe('RedactionBanner', () => {
  it('shows while the view is redacted and turns it off', async () => {
    const { service } = await setup();
    const fixture = TestBed.createComponent(RedactionBanner);
    await settle(fixture);
    expect(fixture.nativeElement.querySelector('[data-testid=redacted-banner]')).toBeNull();
    await service.set(true);
    await settle(fixture);
    buttonByText(fixture, 'Settings…').click();
    await settle(fixture);
    expect(TestBed.inject(DialogService).stack()).toHaveLength(1);
    buttonByText(fixture, 'Turn off').click();
    await settle(fixture);
    expect(service.on()).toBe(false);
    expect(fixture.nativeElement.querySelector('[data-testid=redacted-banner]')).toBeNull();
  });
});
