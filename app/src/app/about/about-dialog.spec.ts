import { signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, settle, text, typeInto } from '../../testing/helpers';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { DialogService } from '../ui/dialog.service';
import { AboutDialog, AboutService, BUILD_INFO } from './about-dialog';
import { bundledLegalSource, LEGAL_SOURCE, LegalDoc, LegalSource } from './legal-source';

const NOTICES = ['# Third-party notices', '- serde 1.0.0 — MIT', '- tokio 1.2.0 — MIT', '- zrender 6.1.0 — BSD-3-Clause'].join('\n');
const TEXTS: Record<LegalDoc, string> = { notices: NOTICES, mit: 'MIT License\nPermission is hereby granted', apache: 'Apache License\nVersion 2.0' };

async function setup(source: LegalSource = { load: async (d) => TEXTS[d] }, status: { version: string } | null = { version: '1.2.3' }) {
  TestBed.configureTestingModule({
    providers: [
      { provide: LEGAL_SOURCE, useValue: source },
      { provide: LifecycleService, useValue: { status: signal(status), platform: signal('linux') } },
    ],
  });
  const fixture = TestBed.createComponent(AboutDialog);
  await settle(fixture);
  return fixture;
}

const rows = (f: Parameters<typeof all>[0]) => all(f, '.line').map((l) => l.textContent);

describe('AboutDialog', () => {
  it('shows the version, the licence and the notices', async () => {
    const f = await setup();
    expect(el(f, '.version').textContent).toBe('1.2.3');
    expect(text(f)).toContain('MIT OR Apache-2.0');
    expect(el(f, '[data-testid=read-only-promise]').textContent).toContain('IMT never modifies your files.');
    expect(rows(f)).toContain('- serde 1.0.0 — MIT');
    expect(el(f, '.viewport').getAttribute('aria-label')).toBe('Third-party notices text');
    expect(text(f)).toContain('4 of 4 lines');
  });

  it('falls back to "unknown" when the engine did not report a version', async () => {
    const f = await setup(undefined, null);
    expect(el(f, '.version').textContent).toBe('unknown');
  });

  it('shows the build info of a build that recorded none as unknown', async () => {
    const f = await setup();
    expect(el(f, '.commit').textContent).toBe('unknown');
    expect(el(f, '.built').textContent).toBe('unknown');
  });

  it('filters the notices as you search and clears the filter', async () => {
    const f = await setup();
    const search = el<HTMLInputElement>(f, 'input[type=search]');
    typeInto(search, 'SERDE');
    await settle(f);
    expect(rows(f)).toEqual(['- serde 1.0.0 — MIT']);
    expect(text(f)).toContain('1 of 4 lines');
    typeInto(search, '');
    await settle(f);
    expect(rows(f)).toHaveLength(4);
  });

  it('virtualises long texts: renders only a window of rows', async () => {
    const big = Array.from({ length: 5000 }, (_, i) => `line ${i}`).join('\n');
    const f = await setup({ load: async () => big });
    expect(all(f, '.line').length).toBeLessThan(60);
    expect(el(f, '.spacer').style.height).toBe('90000px');
    const vp = el(f, '.viewport');
    vp.scrollTop = 18 * 2000;
    vp.dispatchEvent(new Event('scroll'));
    await settle(f);
    expect(rows(f)).toContain('line 2000');
    expect(rows(f)).not.toContain('line 0');
  });

  it('shows LICENSE-MIT and LICENSE-APACHE and goes back to the notices', async () => {
    const f = await setup();
    buttonByText(f, 'View LICENSE-MIT').click();
    await settle(f);
    expect(rows(f)).toEqual(['MIT License', 'Permission is hereby granted']);
    expect(buttonByText(f, 'View LICENSE-MIT').getAttribute('aria-pressed')).toBe('true');
    buttonByText(f, 'View LICENSE-APACHE').click();
    await settle(f);
    expect(rows(f)).toEqual(['Apache License', 'Version 2.0']);
    buttonByText(f, 'Third-party notices').click();
    await settle(f);
    expect(rows(f)).toHaveLength(4);
  });

  it('reports a load failure and offers retry', async () => {
    let fail = true;
    const f = await setup({ load: async (d) => (fail ? Promise.reject(new Error('x')) : TEXTS[d]) });
    expect(el(f, '[role=alert]').textContent).toContain('Could not load Third-party notices.');
    fail = false;
    buttonByText(f, 'Retry').click();
    await settle(f);
    expect(rows(f)).toHaveLength(4);
    expect(all(f, '[role=alert]')).toHaveLength(0);
  });

  it('copies the version info', async () => {
    const writeText = vi.fn(async () => undefined);
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    const f = await setup();
    buttonByText(f, 'Copy version info').click();
    await settle(f);
    expect(writeText).toHaveBeenCalledWith('IMT 1.2.3\nLicence: MIT OR Apache-2.0\nPlatform: linux');
    expect(text(f)).toContain('Copied');
    vi.unstubAllGlobals();
  });

  it('says so when the clipboard refuses', async () => {
    vi.stubGlobal('navigator', { clipboard: { writeText: vi.fn(async () => Promise.reject(new Error('denied'))) } });
    const f = await setup();
    buttonByText(f, 'Copy version info').click();
    await settle(f);
    expect(text(f)).toContain('Copy failed');
    vi.unstubAllGlobals();
  });

  it('never renders file text as HTML', async () => {
    const f = await setup({ load: async () => '<img src=x onerror=alert(1)>' });
    expect(all(f, 'img')).toHaveLength(0);
    expect(rows(f)).toEqual(['<img src=x onerror=alert(1)>']);
  });
});

describe('AboutDialog build info', () => {
  it('shows commit and date when provided and copies them', async () => {
    const writeText = vi.fn(async () => undefined);
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    TestBed.configureTestingModule({
      providers: [
        { provide: BUILD_INFO, useValue: { commit: 'abc1234', date: '2026-10-17' } },
        { provide: LEGAL_SOURCE, useValue: { load: async () => 'x' } },
        { provide: LifecycleService, useValue: { status: signal({ version: '1.0.0' }), platform: signal(null) } },
      ],
    });
    const f = TestBed.createComponent(AboutDialog);
    await settle(f);
    expect(el(f, '.commit').textContent).toBe('abc1234');
    expect(el(f, '.built').textContent).toBe('2026-10-17');
    buttonByText(f, 'Copy version info').click();
    await settle(f);
    expect(writeText).toHaveBeenCalledWith('IMT 1.0.0\nLicence: MIT OR Apache-2.0\nCommit: abc1234\nBuilt: 2026-10-17');
    vi.unstubAllGlobals();
  });
});

describe('AboutService and the bundled sources', () => {
  it('opens one About dialog', () => {
    const dialogs = TestBed.inject(DialogService);
    const svc = TestBed.inject(AboutService);
    svc.open();
    svc.open();
    expect(dialogs.stack().filter((d) => d.component === AboutDialog)).toHaveLength(1);
    expect(dialogs.stack()[0]?.options.title).toBe('About IMT');
  });

  it('loads the generated notices and both licences', async () => {
    expect(await bundledLegalSource.load('notices')).toContain('# Third-party notices');
    expect(await bundledLegalSource.load('mit')).toContain('MIT License');
    expect(await bundledLegalSource.load('apache')).toContain('Apache License');
  });
});
