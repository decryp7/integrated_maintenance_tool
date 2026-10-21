import { CmdError } from '../../core/cmd-error';
import { MockPluginPackagesBackend, MockPluginPackagesOptions, packagesOptionsFromSearch } from './mock-plugin-packages-backend';

const OPTIONS: MockPluginPackagesOptions = { developerMode: false, safeMode: false, damaged: false, variant: '' };
const make = (o: Partial<MockPluginPackagesOptions> = {}) => new MockPluginPackagesBackend({ ...OPTIONS, ...o });

describe('MockPluginPackagesBackend', () => {
  it('reads its switches from the URL', () => {
    expect(packagesOptionsFromSearch('')).toEqual(OPTIONS);
    expect(packagesOptionsFromSearch('?mock-plugin-dev=1&mock-safe=1&mock-plugin-damaged=1&mock-plugin-package=unsigned')).toEqual({
      developerMode: true,
      safeMode: true,
      damaged: true,
      variant: 'unsigned',
    });
    expect(new MockPluginPackagesBackend().options).toEqual(OPTIONS);
  });

  it('starts with one trusted running plugin and reports the modes', async () => {
    const list = await make({ developerMode: true, safeMode: true }).listPackages();
    expect(list.plugins.map((p) => [p.id, p.signature.kind, p.state])).toEqual([['com.contoso.sap-logs', 'trusted', 'running']]);
    expect(list.damaged).toEqual([]);
    expect(list.developerMode).toBe(true);
    expect(list.safeMode).toBe(true);
    expect((await make({ damaged: true }).listPackages()).damaged.map((d) => d.id)).toEqual(['org.example.broken']);
  });

  it('reviews a trusted package with a digest stable per path', async () => {
    const b = make();
    const a = await b.readPackage('/p/web.imtplugin');
    expect(a.signature).toEqual({ kind: 'trusted', signer: 'Example Software Inc.' });
    expect(a.refusal).toBeNull();
    expect(a.digest).toHaveLength(64);
    expect((await b.readPackage('/p/web.imtplugin')).digest).toBe(a.digest);
    expect((await b.readPackage('/q/web.IMTPLUGIN')).digest).not.toBe(a.digest);
    expect(b.calls).toContain('inspect:/p/web.imtplugin');
  });

  it('rejects other files and tampered packages with typed errors', async () => {
    const b = make();
    await expect(b.readPackage('/p/app.log')).rejects.toMatchObject({ code: 'invalid' });
    const err = await b.readPackage('/p/tampered.imtplugin').catch((e: unknown) => e);
    expect(err).toBeInstanceOf(CmdError);
    expect(err).toMatchObject({ code: 'signatureInvalid', detail: 'tampered' });
    await expect(make({ variant: 'tampered' }).readPackage('/p/any.imtplugin')).rejects.toMatchObject({ detail: 'tampered' });
  });

  it('refuses unsigned and unknown-key packages unless in developer mode', async () => {
    const unsigned = await make().readPackage('/p/unsigned.imtplugin');
    expect(unsigned.signature.kind).toBe('unsigned');
    expect(unsigned.refusal?.reason).toBe('unsigned');
    expect(unsigned.warning).toBeNull();
    const unknown = await make().readPackage('/p/unknown-key.imtplugin');
    expect(unknown.signature).toEqual({ kind: 'unknownKey', signer: '7f3a9c21' });
    expect(unknown.refusal?.reason).toBe('unknownKey');
    const dev = await make({ developerMode: true }).readPackage('/p/unsigned.imtplugin');
    expect(dev.refusal).toBeNull();
    expect(dev.warning).toContain('Developer mode');
    expect(dev.developerMode).toBe(true);
    await expect(make().installPackage('/p/unsigned.imtplugin', unsigned.digest, [])).rejects.toMatchObject({ detail: 'unsigned' });
  });

  it('reviews an update of the installed plugin with the added permission', async () => {
    const b = make();
    const u = await b.readPackage('/p/update.imtplugin');
    expect(u.update).toEqual({ fromVersion: '1.2.0', added: ['runPrograms'], removed: [] });
    await b.removePackage('com.contoso.sap-logs');
    expect((await b.readPackage('/p/update.imtplugin')).update).toBeNull();
  });

  it('installs with the approved permissions, pending a restart, and replaces an update', async () => {
    const b = make();
    const r = await b.readPackage('/p/web.imtplugin');
    await expect(b.installPackage('/p/web.imtplugin', 'bad', [])).rejects.toMatchObject({ code: 'invalid' });
    const installed = await b.installPackage('/p/web.imtplugin', r.digest, ['network', 'writeFolders']);
    expect(installed.granted).toEqual(['network']);
    expect(installed.state).toBe('pendingRestart');
    expect(b.calls).toContain(`install:/p/web.imtplugin:${r.digest}:network,writeFolders`);
    const u = await b.readPackage('/p/update.imtplugin');
    await b.installPackage('/p/update.imtplugin', u.digest, ['runPrograms']);
    const list = await b.listPackages();
    expect(list.plugins.map((p) => `${p.id}@${p.version}`)).toEqual(['net.example.web-enricher@2.0.1', 'com.contoso.sap-logs@1.3.0']);
  });

  it('enables, disables, changes permissions and removes', async () => {
    const b = make({ damaged: true });
    expect((await b.setPackageEnabled('com.contoso.sap-logs', false)).state).toBe('disabled');
    expect((await b.setPackageEnabled('com.contoso.sap-logs', true)).state).toBe('pendingRestart');
    expect((await b.setPackagePermissions('com.contoso.sap-logs', ['network'])).granted).toEqual(['network']);
    await expect(b.setPackagePermissions('com.contoso.sap-logs', ['runPrograms'])).rejects.toMatchObject({ field: 'granted' });
    await expect(b.setPackageEnabled('nope', true)).rejects.toMatchObject({ code: 'notFound' });
    b.setState('com.contoso.sap-logs', 'quarantined', 'crashed 3 times');
    b.setState('other', 'ready');
    expect((await b.listPackages()).plugins[0]).toMatchObject({ state: 'quarantined', problem: 'crashed 3 times' });
    expect(await b.removePackage('org.example.broken')).toBe(true);
    expect(await b.removePackage('com.contoso.sap-logs')).toBe(true);
    expect(await b.removePackage('com.contoso.sap-logs')).toBe(false);
    expect(await b.listPackages()).toMatchObject({ plugins: [], damaged: [] });
  });
});
