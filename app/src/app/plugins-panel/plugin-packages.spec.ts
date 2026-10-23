import { ComponentFixture, TestBed } from '@angular/core/testing';
import { all, buttonByText, el, settle, text } from '../../testing/helpers';
import { CmdError } from '../core/cmd-error';
import { DROPPED_FILES_HANDLERS } from '../core/dropped-files';
import { Platform } from '../core/platform';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { mockPackages, mockTools, provideToolsTesting } from '../tools/mock/test-support';
import { MockPluginPackagesOptions } from './mock/mock-plugin-packages-backend';
import { DialogService } from '../ui/dialog.service';
import { PluginsInstalled } from './plugins-installed';
import { permissionTitle, refusalLiftedByTrust, signatureText, stateLabel } from './package-labels';
import { pluginPackageDropHandler } from './package-drop';
import { PackageReviewDialog, PackageReviewOpener } from './package-review-dialog';
import { isPackagePath, PluginPackagesStore } from './plugin-packages.store';
import { PluginPermissionsDialog } from './plugin-permissions-dialog';
import { DialogHost } from '../ui/dialog-host';
import { at } from '../../testing/must';

const SAP = 'com.contoso.sap-logs';

function setup(packages: Partial<MockPluginPackagesOptions> = {}, pick: string | null = '/p/web.imtplugin') {
  const pickOpenPath = vi.fn(async () => pick);
  const showView = vi.fn();
  TestBed.configureTestingModule({
    providers: [
      ...provideToolsTesting({}, packages),
      { provide: Platform, useValue: { isTauri: () => false, pickOpenPath, onFilesDropped: async () => () => undefined, pickPaths: async () => [] } },
      { provide: COMMAND_CONTEXT, useValue: { showView } },
      { provide: DROPPED_FILES_HANDLERS, multi: true, useFactory: pluginPackageDropHandler },
    ],
  });
  return { store: TestBed.inject(PluginPackagesStore), backend: mockPackages(), dialogs: TestBed.inject(DialogService), pickOpenPath, showView };
}

/** Ticks or unticks a checkbox as a user does (`click()` on a checkbox in the test DOM does not fire `change`). */
function toggle(input: HTMLInputElement) {
  input.checked = !input.checked;
  input.dispatchEvent(new Event('change', { bubbles: true }));
}

/** Answers the confirmation on top of the dialog stack. */
function answerTop(dialogs: DialogService, yes: boolean) {
  const top = dialogs.stack().at(-1);
  if (!top) throw new Error('no dialog open');
  top.ref.close(yes ? true : undefined);
}

describe('package labels', () => {
  it('names permissions, states and signatures', () => {
    expect(['openedFilePaths', 'runPrograms', 'writeFolders', 'network'].map((p) => permissionTitle(p as never))).toEqual([
      'Read opened files',
      'Run programs',
      'Write to folders',
      'Network access',
    ]);
    expect((['ready', 'running', 'quarantined', 'disabled', 'pendingRestart', 'unavailable'] as const).map(stateLabel)).toEqual([
      'ready',
      'running',
      'quarantined',
      'disabled',
      'pending restart',
      'unavailable',
    ]);
    expect(signatureText({ kind: 'trusted', signer: 'Contoso' })).toBe('Signed by Contoso');
    expect(signatureText({ kind: 'trusted', signer: null })).toBe('Signed by ?');
    expect(signatureText({ kind: 'unknownKey', signer: 'ab12' })).toBe('Signed with an unknown key ab12');
    expect(signatureText({ kind: 'unknownKey', signer: null })).toContain('?');
    expect(signatureText({ kind: 'unsigned', signer: null })).toBe('Not signed');
    expect(refusalLiftedByTrust('unsigned')).toBe(true);
    expect(refusalLiftedByTrust('unknownKey')).toBe(true);
    expect(refusalLiftedByTrust('wrongTarget')).toBe(false);
    expect(isPackagePath('/a/b.IMTPLUGIN')).toBe(true);
    expect(isPackagePath('/a/b.imtplugin.log')).toBe(false);
  });
});

describe('PluginPackagesStore', () => {
  it('loads the installed plugins and the modes', async () => {
    const { store } = setup({ developerMode: true, safeMode: true, damaged: true });
    expect(store.plugins()).toEqual([]);
    expect(store.damaged()).toEqual([]);
    await store.load();
    expect(store.plugins().map((p) => p.id)).toEqual([SAP]);
    expect(store.damaged().length).toBe(1);
    expect(store.developerMode()).toBe(true);
    expect(store.safeMode()).toBe(true);
    expect(store.showRestart()).toBe(false);
    expect(store.loading()).toBe(false);
  });

  it('keeps a failed load as a retryable error, and Retry clears it', async () => {
    const { store, backend } = setup();
    vi.spyOn(backend, 'listPackages').mockRejectedValueOnce(new CmdError({ code: 'io', message: 'Cannot read the plugins folder.', retryable: true, hint: 'Check it.' }));
    await store.load();
    expect(store.error()).toBe('Cannot read the plugins folder. Check it.');
    expect(store.retry()).toBe('load');
    await store.retryFailed();
    expect(store.error()).toBeNull();
    expect(store.plugins().length).toBe(1);
    // Nothing to retry: a no-op.
    await store.retryFailed();
    expect(store.error()).toBeNull();
  });

  it('picks a package in the dialog filtered to .imtplugin and reviews it with every permission approved', async () => {
    const { store, pickOpenPath, backend } = setup();
    await store.pickAndReview();
    expect(pickOpenPath).toHaveBeenCalledWith('Install a plugin', [{ name: 'IMT plugin package', extensions: ['imtplugin'] }]);
    expect(backend.calls).toContain('inspect:/p/web.imtplugin');
    expect(store.review()?.path).toBe('/p/web.imtplugin');
    expect(store.review()?.approved).toEqual(['runPrograms', 'network']);
    expect(store.canInstall()).toBe(true);
  });

  it('does nothing when the dialog is cancelled, and reports a failing dialog', async () => {
    const { store, pickOpenPath, backend } = setup({}, null);
    await store.pickAndReview();
    expect(store.review()).toBeNull();
    expect(backend.calls).toEqual([]);
    pickOpenPath.mockRejectedValueOnce(new Error('dialog unavailable'));
    await store.pickAndReview();
    expect(store.error()).toBe('dialog unavailable');
    expect(store.retry()).toBeNull();
  });

  it('toggles approved permissions (and ignores toggles without a review)', async () => {
    const { store } = setup();
    store.toggleApproved('network');
    expect(store.review()).toBeNull();
    await store.reviewPackage('/p/web.imtplugin');
    store.toggleApproved('network');
    expect(store.review()?.approved).toEqual(['runPrograms']);
    store.toggleApproved('network');
    expect(store.review()?.approved).toEqual(['runPrograms', 'network']);
  });

  it('installs with the reviewed digest and approved permissions, closes the review and needs a restart', async () => {
    const { store, backend } = setup();
    await store.load();
    await store.reviewPackage('/p/web.imtplugin');
    const digest = store.review()?.pkg.digest;
    store.toggleApproved('runPrograms');
    await store.install();
    expect(backend.calls).toContain(`install:/p/web.imtplugin:${digest}:network`);
    expect(store.review()).toBeNull();
    expect(store.restartNeeded()).toBe(true);
    expect(store.showRestart()).toBe(true);
    expect(store.plugins().map((p) => p.id)).toEqual([SAP, 'net.example.web-enricher']);
    await store.restart();
    expect(mockTools().calls.some((c) => c.startsWith('restartApp:false'))).toBe(true);
  });

  it('keeps the review open with the error when installing fails, and never installs a refused package', async () => {
    const { store, backend } = setup();
    await store.install();
    expect(backend.calls).toEqual([]);
    await store.reviewPackage('/p/unsigned.imtplugin');
    expect(store.review()?.approved).toEqual([]);
    expect(store.canInstall()).toBe(false);
    await store.install();
    expect(backend.calls.some((c) => c.startsWith('install:'))).toBe(false);
    await store.reviewPackage('/p/web.imtplugin');
    vi.spyOn(backend, 'installPackage').mockRejectedValueOnce(new CmdError({ code: 'diskFull', message: 'The disk is full.', retryable: true }));
    await store.install();
    expect(store.review()).not.toBeNull();
    expect(store.error()).toBe('The disk is full.');
    expect(store.restartNeeded()).toBe(false);
    store.cancelReview();
    expect(store.review()).toBeNull();
    expect(store.error()).toBeNull();
  });

  it('reports a tampered package without opening a review', async () => {
    const { store } = setup();
    await store.reviewPackage('/p/tampered.imtplugin');
    expect(store.review()).toBeNull();
    expect(store.error()).toBe('The package was changed after it was signed. Get the package again from its publisher.');
    store.dismissError();
    expect(store.error()).toBeNull();
  });

  it('enables, disables and changes permissions in place', async () => {
    const { store, backend } = setup();
    await store.setEnabled(SAP, false);
    expect(store.plugins()).toEqual([]);
    await store.load();
    await store.setEnabled(SAP, false);
    expect(store.plugins()[0]?.state).toBe('disabled');
    await store.setEnabled(SAP, true);
    expect(store.plugins()[0]?.state).toBe('pendingRestart');
    expect(store.showRestart()).toBe(true);
    expect(await store.setPermissions(SAP, ['network'])).toBe(true);
    expect(store.plugins()[0]?.granted).toEqual(['network']);
    expect(backend.calls).toEqual([`setEnabled:${SAP}:false`, `setEnabled:${SAP}:false`, `setEnabled:${SAP}:true`, `setPermissions:${SAP}:network`]);
    expect(await store.setPermissions(SAP, ['writeFolders'])).toBe(false);
    expect(store.error()).toContain('does not ask for writeFolders');
  });

  it('removes after confirming, and keeps the plugin when the user says no', async () => {
    const { store, backend, dialogs } = setup();
    await store.load();
    const no = store.remove(SAP);
    expect(dialogs.stack().at(-1)?.options.inputs?.['message']).toContain('Remove SAP log importer?');
    answerTop(dialogs, false);
    await no;
    expect(backend.calls).toEqual([]);
    const yes = store.remove(SAP);
    answerTop(dialogs, true);
    await yes;
    expect(backend.calls).toEqual([`remove:${SAP}`]);
    expect(store.plugins()).toEqual([]);
    // It was running: gone after a restart.
    expect(store.restartNeeded()).toBe(true);
  });

  it('removes a damaged record without asking for a restart, and reports a failed removal', async () => {
    const { store, backend, dialogs } = setup({ damaged: true });
    await store.load();
    const broken = store.remove('org.example.broken');
    expect(dialogs.stack().at(-1)?.options.inputs?.['message']).toContain('org.example.broken');
    answerTop(dialogs, true);
    await broken;
    expect(store.damaged()).toEqual([]);
    expect(store.restartNeeded()).toBe(false);
    vi.spyOn(backend, 'removePackage').mockRejectedValueOnce(new Error('file in use'));
    const failing = store.remove(SAP);
    answerTop(dialogs, true);
    await failing;
    expect(store.error()).toBe('file in use');
    expect(store.plugins().length).toBe(1);
  });

  it('resets a quarantine through the lifecycle backend and reloads', async () => {
    const { store, backend } = setup();
    backend.setState(SAP, 'quarantined', 'crashed');
    await store.load();
    const reload = vi.spyOn(backend, 'listPackages');
    await store.reset(SAP);
    expect(mockTools().calls).toContain(`pluginReset:${SAP}`);
    expect(reload).toHaveBeenCalled();
    vi.spyOn(mockTools(), 'pluginReset').mockRejectedValueOnce(new Error('not quarantined'));
    reload.mockClear();
    await store.reset(SAP);
    expect(store.error()).toBe('not quarantined');
    expect(reload).not.toHaveBeenCalled();
  });
});

describe('dropped plugin packages', () => {
  it('claims .imtplugin files, shows the Plugins panel and reviews the first', async () => {
    const { store, showView } = setup();
    const [drop] = TestBed.inject(DROPPED_FILES_HANDLERS);
    expect(await drop!(['/l/app.log'])).toEqual([]);
    expect(showView).not.toHaveBeenCalled();
    expect(await drop!(['/l/app.log', '/p/web.imtplugin', '/p/update.IMTPLUGIN'])).toEqual(['/p/web.imtplugin', '/p/update.IMTPLUGIN']);
    expect(showView).toHaveBeenCalledWith('plugins');
    expect(store.review()?.path).toBe('/p/web.imtplugin');
    // The review dialog opened (its effect runs on the next tick).
    await settle();
    expect(TestBed.inject(DialogService).stack().map((d) => d.component)).toEqual([PackageReviewDialog]);
  });
});

async function hostFixture() {
  const host = TestBed.createComponent(DialogHost);
  await settle(host);
  return host;
}

describe('PackageReviewDialog', () => {
  it('shows publisher, signature, permissions with reasons, and installs the approved ones', async () => {
    const { store, backend, dialogs } = setup();
    TestBed.inject(PackageReviewOpener);
    const host = await hostFixture();
    await store.reviewPackage('/p/web.imtplugin');
    await settle(host);
    expect(dialogs.stack().length).toBe(1);
    expect(el(host, '[role=dialog] h2').textContent).toBe('Install plugin');
    expect(el(host, '[data-testid=package-name]').textContent).toBe('Web access log enricher');
    expect(el(host, '[data-testid=package-publisher]').textContent).toBe('Example Software');
    const sig = el(host, '[data-testid=package-signature]');
    expect(sig.textContent).toBe('Signed by Example Software Inc.');
    expect(sig.classList).toContain('trusted');
    expect(text(host)).toContain('Publisher\'s reason: To download the GeoIP database.');
    expect(text(host)).toContain('Connect to servers on the network or the internet.');
    const boxes = all<HTMLInputElement>(host, '.perm input');
    expect(boxes.map((b) => b.checked)).toEqual([true, true]);
    at(boxes, 1).click();
    await settle(host);
    expect(store.review()?.approved).toEqual(['runPrograms']);
    buttonByText(host, 'Install').click();
    await settle(host);
    expect(backend.calls.some((c) => c.endsWith(':runPrograms') && c.startsWith('install:'))).toBe(true);
    // Installed: the dialog closed itself.
    expect(dialogs.stack()).toEqual([]);
    expect(store.review()).toBeNull();
  });

  it('cancels the review when closed', async () => {
    const { store, dialogs } = setup();
    TestBed.inject(PackageReviewOpener);
    const host = await hostFixture();
    await store.reviewPackage('/p/web.imtplugin');
    await settle(host);
    buttonByText(host, 'Cancel').click();
    await settle(host);
    expect(dialogs.stack()).toEqual([]);
    expect(store.review()).toBeNull();
    // A new review opens a new dialog.
    await store.reviewPackage('/p/web.imtplugin');
    await settle(host);
    expect(dialogs.stack().length).toBe(1);
    dialogs.dismissTop();
    await settle(host);
    expect(store.review()).toBeNull();
  });

  it('shows the refusal with the trust hint and disables Install', async () => {
    const { store } = setup();
    TestBed.inject(PackageReviewOpener);
    const host = await hostFixture();
    await store.reviewPackage('/p/unsigned.imtplugin');
    await settle(host);
    expect(el(host, '[data-testid=package-signature]').classList).toContain('unsigned');
    expect(el(host, '[data-testid=package-refusal]').textContent).toContain('not signed');
    expect(text(host)).toContain('Settings → Plugin trust');
    expect(buttonByText(host, 'Install').disabled).toBe(true);
    expect(el<HTMLFieldSetElement>(host, 'fieldset').disabled).toBe(true);
  });

  it('shows a refusal that trust cannot lift without the hint', async () => {
    const { store, backend } = setup();
    TestBed.inject(PackageReviewOpener);
    const host = await hostFixture();
    const real = await backend.readPackage('/p/web.imtplugin');
    vi.spyOn(backend, 'readPackage').mockResolvedValueOnce({ ...real, permissions: [], refusal: { reason: 'wrongTarget', message: 'Built for macOS only.' } });
    await store.reviewPackage('/p/web.imtplugin');
    await settle(host);
    expect(el(host, '[data-testid=package-refusal]').textContent).toContain('Built for macOS only.');
    expect(text(host)).not.toContain('Plugin trust');
    expect(text(host)).toContain('asks for no permissions');
  });

  it('shows the developer-mode warning and an update with its added permission', async () => {
    const { store } = setup({ developerMode: true });
    TestBed.inject(PackageReviewOpener);
    const host = await hostFixture();
    await store.reviewPackage('/p/unsigned.imtplugin');
    await settle(host);
    expect(el(host, '[data-testid=package-warning]').textContent).toContain('Developer mode');
    expect(buttonByText(host, 'Install').disabled).toBe(false);
    store.cancelReview();
    await settle(host);
    await store.reviewPackage('/p/update.imtplugin');
    await settle(host);
    const update = el(host, '[data-testid=package-update]').textContent ?? '';
    expect(update).toContain('Replaces version 1.2.0.');
    expect(update).toContain('Newly requested: Run programs');
    expect(all(host, '.badge.new').length).toBe(1);
    expect(buttonByText(host, 'Update')).toBeTruthy();
    const real = store.review()!;
    store.cancelReview();
    await settle(host);
    const { backend } = { backend: mockPackages() };
    vi.spyOn(backend, 'readPackage').mockResolvedValueOnce({ ...real.pkg, update: { fromVersion: '1.0', added: [], removed: ['network'] } });
    await store.reviewPackage('/p/update.imtplugin');
    await settle(host);
    expect(el(host, '[data-testid=package-update]').textContent).toContain('No longer requested: Network access');
  });

  it('shows an install failure inline', async () => {
    const { store, backend } = setup();
    TestBed.inject(PackageReviewOpener);
    const host = await hostFixture();
    await store.reviewPackage('/p/web.imtplugin');
    await settle(host);
    vi.spyOn(backend, 'installPackage').mockRejectedValueOnce(new Error('The package changed since you reviewed it.'));
    buttonByText(host, 'Install').click();
    await settle(host);
    expect(el(host, '[role=dialog] .err').textContent).toContain('changed since you reviewed it');
    el<HTMLButtonElement>(host, '[role=dialog] .err .btn.ghost').click();
    await settle(host);
    expect(store.error()).toBeNull();
  });
});

describe('InstalledPackages', () => {
  async function view(packages: Partial<MockPluginPackagesOptions> = {}) {
    const ctx = setup(packages);
    const host = await hostFixture();
    const fixture = TestBed.createComponent(PluginsInstalled);
    await settle(fixture);
    return { ...ctx, fixture, host };
  }

  const row = (fixture: ComponentFixture<unknown>, id: string) => el(fixture, `tr.plugin[data-id="${id}"]`);
  const rowMenu = async (fixture: ComponentFixture<unknown>, id: string) => {
    row(fixture, id).querySelector<HTMLButtonElement>('[data-testid=plugin-more]')!.click();
    await settle(fixture);
  };
  const menuItem = (testid: string) => document.body.querySelector<HTMLButtonElement>(`[data-testid=${testid}]`)!;

  it('lists installed packages as rows of the plugins table, with their signature and state in the side panel', async () => {
    const { fixture } = await view();
    const tr = row(fixture, SAP);
    expect(tr.textContent).toContain('SAP log importer');
    expect(tr.textContent).toContain('1.2.0');
    expect(tr.textContent).toContain('Contoso');
    expect(tr.querySelector('[data-testid=plugin-source]')!.textContent?.trim()).toBe('Package');
    expect(all(fixture, '.package')).toHaveLength(0);
    tr.click();
    await settle(fixture);
    expect(el(fixture, '[data-testid=package-signature]').textContent).toBe('Signed by Contoso Ltd');
    expect(el(fixture, '[data-testid=package-state]').textContent).toBe('running');
    expect(el(fixture, '[data-testid=plugin-permissions]').textContent).toContain('Network access');
    expect(text(fixture)).not.toContain('Changes apply after a restart');
  });

  it('installs from a file through the review, then offers the restart', async () => {
    const { fixture, host, store, backend } = await view();
    buttonByText(fixture, 'Install from file…').click();
    await settle(fixture);
    await settle(host);
    buttonByText(host, 'Install').click();
    await settle(host);
    await settle(fixture);
    expect(store.plugins().length).toBe(2);
    expect(all(fixture, 'tr.plugin [data-testid=plugin-source]').filter((e) => e.textContent?.includes('Package'))).toHaveLength(2);
    expect(row(fixture, 'net.example.web-enricher').textContent).toContain('Not running');
    expect(el(fixture, '[data-testid=packages-restart]').textContent).toContain('Changes apply after a restart.');
    buttonByText(fixture, 'Restart to apply').click();
    await settle(fixture);
    expect(mockTools().calls.some((c) => c.startsWith('restartApp:'))).toBe(true);
    expect(backend.calls.some((c) => c.startsWith('install:'))).toBe(true);
  });

  it('disables, enables, revokes a permission and removes with confirmation', async () => {
    const { fixture, host, store, backend, dialogs } = await view();
    await rowMenu(fixture, SAP);
    menuItem('plugin-disable').click();
    await settle(fixture);
    expect(store.plugins()[0]?.state).toBe('disabled');
    await rowMenu(fixture, SAP);
    menuItem('plugin-enable').click();
    await settle(fixture);
    expect(store.plugins()[0]?.state).toBe('pendingRestart');

    row(fixture, SAP).click();
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[data-testid=plugin-permissions-edit]').click();
    await settle(host);
    expect(el(host, '[role=dialog] h2').textContent).toBe('Permissions of SAP log importer');
    expect(text(host)).toContain('restarts at once with the new permissions');
    const apply = buttonByText(host, 'Apply and restart plugin');
    expect(apply.disabled).toBe(true);
    const network = () => el<HTMLInputElement>(host, 'input[data-permission=network]');
    toggle(network());
    await settle(host);
    toggle(network());
    await settle(host);
    expect(apply.disabled).toBe(true);
    toggle(network());
    await settle(host);
    expect(apply.disabled).toBe(false);
    apply.click();
    await settle(host);
    await settle(fixture);
    expect(backend.calls).toContain(`setPermissions:${SAP}:`);
    expect(dialogs.stack()).toEqual([]);
    expect(store.plugins()[0]?.granted).toEqual([]);

    await rowMenu(fixture, SAP);
    menuItem('plugin-remove').click();
    await settle(host);
    expect(el(host, '[data-testid=confirm-message]').textContent).toContain('Remove SAP log importer?');
    buttonByText(host, /^Remove$/).click();
    await settle(host);
    await settle(fixture);
    expect(backend.calls).toContain(`remove:${SAP}`);
    expect(store.plugins()).toEqual([]);
  });

  it('keeps the permissions dialog open with the error when applying fails', async () => {
    const { fixture, host, backend, dialogs } = await view();
    row(fixture, SAP).click();
    await settle(fixture);
    el<HTMLButtonElement>(fixture, '[data-testid=plugin-permissions-edit]').click();
    await settle(host);
    vi.spyOn(backend, 'setPackagePermissions').mockRejectedValueOnce(new Error('the plugin did not restart'));
    toggle(el<HTMLInputElement>(host, 'input[data-permission=network]'));
    await settle(host);
    buttonByText(host, 'Apply and restart plugin').click();
    await settle(host);
    expect(dialogs.stack().length).toBe(1);
    expect(el(host, '[role=dialog] .err').textContent).toContain('the plugin did not restart');
    buttonByText(host, 'Cancel').click();
    await settle(host);
    expect(dialogs.stack()).toEqual([]);
  });

  it('says when the plugin of the permissions dialog is gone', async () => {
    const { host, dialogs } = await view();
    dialogs.open(PluginPermissionsDialog, { title: 'P', inputs: { pluginId: 'gone' } });
    await settle(host);
    expect(text(host)).toContain('no longer installed');
  });

  it('shows a quarantined package with its problem and resets it from the row menu', async () => {
    const ctx = setup();
    ctx.backend.setState(SAP, 'quarantined', 'Crashed 3 times in a row.');
    const fixture = TestBed.createComponent(PluginsInstalled);
    await settle(fixture);
    expect(row(fixture, SAP).querySelector('[data-testid=plugin-status]')!.getAttribute('data-status')).toBe('quarantined');
    row(fixture, SAP).click();
    await settle(fixture);
    expect(el(fixture, '[data-testid=package-problem]').textContent).toBe('Crashed 3 times in a row.');
    await rowMenu(fixture, SAP);
    menuItem('plugin-reset').click();
    await settle(fixture);
    expect(mockTools().calls).toContain(`pluginReset:${SAP}`);
  });

  it('shows safe mode, damaged records and a load failure with Retry', async () => {
    const ctx = setup({ safeMode: true, damaged: true });
    const reject = vi.spyOn(ctx.backend, 'listPackages').mockRejectedValueOnce(new Error('plugins folder unreadable'));
    const fixture = TestBed.createComponent(PluginsInstalled);
    await settle(fixture);
    expect(text(fixture)).toContain('plugins folder unreadable');
    buttonByText(fixture, 'Retry').click();
    await settle(fixture);
    expect(reject).toHaveBeenCalledTimes(2);
    expect(text(fixture)).not.toContain('plugins folder unreadable');
    expect(el(fixture, '[data-testid=packages-safe-mode]').textContent).toContain('third-party plugins are not loaded');
    expect(el(fixture, '[data-testid=package-damaged]').textContent).toContain('missing field `version`');
  });

  it('flags an unsigned package in its Source cell and shows the developer-mode warning', async () => {
    const ctx = setup({ developerMode: true });
    const r = await ctx.backend.readPackage('/p/unsigned.imtplugin');
    await ctx.backend.installPackage('/p/unsigned.imtplugin', r.digest, []);
    await ctx.backend.removePackage(SAP);
    const fixture = TestBed.createComponent(PluginsInstalled);
    await settle(fixture);
    const tr = all(fixture, 'tr.plugin').at(-1)!;
    expect(tr.querySelector('[data-testid=package-signature]')!.textContent).toBe('Not signed');
    tr.click();
    await settle(fixture);
    expect(text(fixture)).toContain('Developer mode');
    expect(text(fixture)).toContain('No permissions granted.');
  });
});
