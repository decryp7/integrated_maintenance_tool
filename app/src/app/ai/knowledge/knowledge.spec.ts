import { TestBed } from '@angular/core/testing';
import { all, buttonByText, el, settle, text } from '../../../testing/helpers';
import { at } from '../../../testing/must';
import { backendCallStarted, methodKind } from '../../core/activity/backend-activity';
import { activity } from '../../core/activity/runtime';
import { Platform } from '../../core/platform';
import { DIALOG_REF, DialogService } from '../../ui/dialog.service';
import { AiError } from '../ai-backend';
import { KnowledgeEditor } from './knowledge-editor';
import { KnowledgeBackend, TauriKnowledgeBackend } from './knowledge-backend';
import { KnowledgePersistence } from './knowledge-persistence';
import { KnowledgeSettings } from './knowledge-settings';
import { KnowledgeStore } from './knowledge.store';
import { MockKnowledgeBackend } from './mock-knowledge-backend';

function provide(backend = new MockKnowledgeBackend(), extra: unknown[] = []) {
  TestBed.configureTestingModule({ providers: [{ provide: KnowledgeBackend, useValue: backend }, ...(extra as [])] });
  return backend;
}

describe('TauriKnowledgeBackend', () => {
  it('calls the knowledge commands with their arguments and types rejections', async () => {
    const call = vi.fn().mockResolvedValue({});
    const b = new TauriKnowledgeBackend(call as never);
    await b.listKnowledge();
    await b.getKnowledge('global', 'k1');
    await b.createKnowledge('project', 'Glossary');
    await b.importKnowledge('global', '/a.md');
    await b.updateKnowledge('project', 'k1', { enabled: false });
    await b.removeKnowledge('global', 'k2');
    await b.exportKnowledge('global', 'k2', '/out.md');
    await b.exportKnowledge('global', 'k2', '/out.md', true);
    await b.getProjectKnowledge();
    await b.restoreProjectKnowledge(undefined);
    expect(call.mock.calls).toEqual([
      ['ai_knowledge_list', undefined],
      ['ai_knowledge_get', { scope: 'global', id: 'k1' }],
      ['ai_knowledge_create', { scope: 'project', name: 'Glossary', text: '' }],
      ['ai_knowledge_import', { scope: 'global', path: '/a.md' }],
      ['ai_knowledge_update', { scope: 'project', id: 'k1', change: { enabled: false } }],
      ['ai_knowledge_remove', { scope: 'global', id: 'k2' }],
      ['ai_knowledge_export', { scope: 'global', id: 'k2', path: '/out.md' }],
      ['ai_knowledge_export', { scope: 'global', id: 'k2', path: '/out.md', overwrite: true }],
      ['ai_knowledge_project_record', undefined],
      ['ai_knowledge_project_restore', { record: null }],
    ]);
    call.mockRejectedValueOnce({ code: 'notFound', message: 'gone' });
    await expect(b.getKnowledge('global', 'k9')).rejects.toMatchObject({ code: 'notFound', message: 'gone' });
  });

  it('names its calls in the activity log without names or text', () => {
    expect(['listKnowledge', 'getKnowledge', 'getProjectKnowledge'].map(methodKind)).toEqual(['query', 'query', 'query']);
    expect(methodKind('updateKnowledge')).toBe('mutation');
    activity.reset();
    vi.spyOn(activity.triggers, 'infer').mockReturnValue('menu');
    const begin = vi.spyOn(activity, 'begin');
    backendCallStarted('updateKnowledge', ['global', 'k1', { name: 'Secret name', text: 'SECRET TEXT' }]);
    backendCallStarted('createKnowledge', ['team', 'Secret name', 'SECRET TEXT']);
    const params = JSON.stringify(begin.mock.calls);
    expect(params).toContain('ai.knowledge.change');
    expect(params).toContain('name,text');
    expect(params).not.toContain('SECRET');
    expect(params).not.toContain('Secret name');
    activity.reset();
  });
});

describe('MockKnowledgeBackend', () => {
  it('mimics the engine: ids, checks, restore of damaged records', async () => {
    const m = new MockKnowledgeBackend('?mockKnowledge=1');
    expect(at((await m.listKnowledge()).global, 0).name).toBe('Glossary');
    const a = await m.createKnowledge('project', ' Notes ', 'a\r\nb');
    expect(a).toMatchObject({ id: 'k1', name: 'Notes', bytes: 3 });
    await expect(m.createKnowledge('project', ' ')).rejects.toBeInstanceOf(AiError);
    await expect(m.updateKnowledge('project', 'k1', { text: 'x'.repeat(64 * 1024 + 1) })).rejects.toMatchObject({ code: 'invalid' });
    await expect(m.getKnowledge('project', 'k9')).rejects.toMatchObject({ code: 'notFound' });
    expect((await m.importKnowledge('global', 'C:\\docs\\Runbook.md')).name).toBe('Runbook');
    expect(await m.exportKnowledge('project', 'k1', '/x.md')).toBe('/x.md');
    const rec = await m.getProjectKnowledge();
    expect((await m.restoreProjectKnowledge({ version: 9 })).warning).toContain('damaged');
    expect((await m.restoreProjectKnowledge(rec)).files).toHaveLength(1);
    expect(await m.removeKnowledge('project', 'k1')).toBe(true);
  });
});

describe('KnowledgePersistence', () => {
  it('captures the engine record, restores saved files and keeps them when the engine cannot take them', async () => {
    const backend = provide();
    const p = TestBed.inject(KnowledgePersistence);
    expect(p.contribution.key).toBe('knowledge');
    expect(p.contribution.capture()).toBeNull();
    const record = { version: 1, files: [{ id: 'k1', name: 'Incident', enabled: true, text: 'WEB01 patched' }] };
    await p.restore(record);
    expect(p.contribution.capture()).toEqual(record);
    expect(p.restored()).toBe(1);
    await p.restore({ version: 9 });
    expect(p.warning()).toContain('damaged');
    expect(p.record()).toBeNull();
    vi.spyOn(backend, 'restoreProjectKnowledge').mockRejectedValueOnce(new AiError('unavailable', 'off'));
    await p.restore(record);
    expect(p.record()).toEqual(record);
    vi.spyOn(backend, 'restoreProjectKnowledge').mockRejectedValueOnce(new AiError('unavailable', 'off'));
    await p.restore('junk');
    expect(p.record()).toBeNull();
    vi.spyOn(backend, 'getProjectKnowledge').mockRejectedValueOnce(new AiError('unavailable', 'off'));
    await p.sync();
    p.contribution.reset?.();
    p.contribution.restore(record);
    await settle();
  });
});

describe('KnowledgeStore', () => {
  it('lists, changes and refreshes the project record only for project files', async () => {
    const backend = provide();
    const store = TestBed.inject(KnowledgeStore);
    const sync = vi.spyOn(TestBed.inject(KnowledgePersistence), 'sync');
    await store.load();
    expect(store.project()).toEqual([]);
    const g = await store.create('global', 'Glossary', 'ACME-GW: gateway');
    expect(sync).not.toHaveBeenCalled();
    const p = await store.create('project', 'Incident');
    expect(sync).toHaveBeenCalledTimes(1);
    expect(at(store.global(), 0).name).toBe('Glossary');
    await store.setEnabled(at(store.global(), 0), false);
    expect(at(store.global(), 0).enabled).toBe(false);
    await store.update(at(store.project(), 0), { name: 'Incident 7' });
    expect(at(store.project(), 0).name).toBe('Incident 7');
    expect(await store.text(at(store.global(), 0))).toBe('ACME-GW: gateway');
    expect(await store.importFile('project', '/x/Runbook.md')).toMatchObject({ name: 'Runbook' });
    await store.remove(at(store.project(), 0));
    expect(store.project().map((f) => f.name)).toEqual(['Runbook']);
    expect(g && p).toBeTruthy();
    // A refusal is shown and the list stays.
    expect(await store.create('project', ' ')).toBeNull();
    expect(store.actionError()).toContain('name');
    store.dismissError();
    expect(store.actionError()).toBeNull();
    // A failed listing is shown with its reason.
    vi.spyOn(backend, 'listKnowledge').mockRejectedValueOnce(new AiError('unavailable', 'AI is off'));
    await store.load();
    expect(store.loadError()).toBe('AI is off');
    expect(store.maxFiles()).toBe(32);
  });
});

describe('KnowledgeSettings', () => {
  async function setup() {
    const platform = { pickOpenPath: vi.fn().mockResolvedValue('/docs/Runbook.md'), pickSavePath: vi.fn().mockResolvedValue('/out/Glossary.md'), saveDialogConfirmsOverwrite: () => true };
    const backend = provide(new MockKnowledgeBackend('?mockKnowledge=1'), [{ provide: Platform, useValue: platform }]);
    const f = TestBed.createComponent(KnowledgeSettings);
    await settle(f);
    return { f, backend, platform, dialogs: TestBed.inject(DialogService) };
  }

  it('lists both scopes and toggles, imports and exports files', async () => {
    const { f, backend, platform } = await setup();
    expect(el(f, '[data-testid=knowledge-project]').textContent).toContain('No knowledge files yet.');
    expect(el(f, '[data-testid=knowledge-global]').textContent).toContain('Glossary');
    const box = el<HTMLInputElement>(f, '[data-testid="knowledge-file-Glossary"] input[type=checkbox]');
    expect(box.getAttribute('aria-label')).toBe('The AI reads Glossary');
    box.click();
    await settle(f);
    expect(at((await backend.listKnowledge()).global, 0).enabled).toBe(false);
    expect(text(f)).toContain('not read by the AI');
    el<HTMLButtonElement>(f, '[data-testid=knowledge-import-project]').click();
    await settle(f);
    expect(platform.pickOpenPath).toHaveBeenCalled();
    expect(el(f, '[data-testid=knowledge-project]').textContent).toContain('Runbook');
    buttonByText(f, 'Export…').click();
    await settle(f);
    expect(el(f, '[data-testid=knowledge-notice]').textContent).toContain('Exported to /out/Glossary.md');
    vi.spyOn(backend, 'exportKnowledge').mockRejectedValueOnce(new AiError('io', 'disk full'));
    buttonByText(f, 'Export…').click();
    await settle(f);
    expect(text(f)).toContain('Not exported: disk full');
  });

  it('asks before removing and opens the editor for new and existing files', async () => {
    const { f, dialogs, backend } = await setup();
    buttonByText(f, 'Remove…').click();
    await settle(f);
    let dlg = dialogs.stack().at(-1);
    expect(dlg?.options.title).toBe('Remove knowledge file?');
    expect(String(dlg?.options.inputs?.['message'])).toContain('deleted from this computer');
    dlg?.ref.close();
    await settle(f);
    expect((await backend.listKnowledge()).global).toHaveLength(1);
    buttonByText(f, 'Remove…').click();
    await settle(f);
    dialogs.stack().at(-1)?.ref.close(true);
    await settle(f);
    expect((await backend.listKnowledge()).global).toHaveLength(0);
    el<HTMLButtonElement>(f, '[data-testid=knowledge-new-project]').click();
    dlg = dialogs.stack().at(-1);
    expect(dlg?.options.title).toBe('New knowledge file');
    expect(dlg?.options.inputs).toMatchObject({ file: null, scope: 'project', maxBytes: 65536 });
    dlg?.ref.close();
    await backend.createKnowledge('project', 'Incident');
    TestBed.inject(KnowledgePersistence).restored.set(5);
    await settle(f);
    expect(at(all(f, '[data-testid=knowledge-project] li'), 0).textContent).toContain('Incident');
    buttonByText(f, 'Edit…').click();
    expect(dialogs.stack().at(-1)?.options.title).toBe('Edit knowledge file');
  });

  it('shows warnings, a failed listing with Retry and refusals', async () => {
    const backend = provide();
    vi.spyOn(backend, 'listKnowledge').mockRejectedValueOnce(new AiError('unavailable', 'AI is off'));
    const f = TestBed.createComponent(KnowledgeSettings);
    await settle(f);
    expect(text(f)).toContain('could not be listed: AI is off');
    vi.spyOn(backend, 'listKnowledge').mockResolvedValueOnce({ project: [], global: [], warnings: ['The list was damaged.'], globalReadOnly: true, maxFileBytes: 1, maxFiles: 1 });
    buttonByText(f, 'Retry').click();
    await settle(f);
    expect(text(f)).toContain('The list was damaged.');
    expect(el<HTMLButtonElement>(f, '[data-testid=knowledge-new-global]').disabled).toBe(true);
    expect(el<HTMLButtonElement>(f, '[data-testid=knowledge-new-project]').disabled).toBe(false);
    vi.spyOn(backend, 'importKnowledge').mockRejectedValueOnce(new AiError('limit', 'too large'));
    TestBed.inject(KnowledgeStore).importFile('project', '/big.md');
    await settle(f);
    expect(el(f, '[data-testid=knowledge-error]').textContent).toContain('too large');
  });
});

describe('KnowledgeEditor', () => {
  function open(inputs: Record<string, unknown> = {}) {
    const backend = provide();
    const ref = { close: vi.fn(), closed: new Promise(() => undefined) };
    TestBed.configureTestingModule({ providers: [{ provide: DIALOG_REF, useValue: ref }] });
    const f = TestBed.createComponent(KnowledgeEditor);
    for (const [k, v] of Object.entries(inputs)) f.componentRef.setInput(k, v);
    return { f, ref, backend };
  }
  const type = (f: Parameters<typeof el>[0], sel: string, v: string) => {
    const i = el<HTMLInputElement>(f, sel);
    i.value = v;
    i.dispatchEvent(new Event('input'));
  };

  it('writes a new file, previews it as text and saves it through the engine', async () => {
    const { f, ref, backend } = open({ scope: 'project' });
    await settle(f);
    const save = el<HTMLButtonElement>(f, '[data-testid=knowledge-save]');
    expect(save.disabled).toBe(true);
    type(f, '[data-testid=knowledge-name]', 'Glossary');
    type(f, '[data-testid=knowledge-text]', '# Names\n\n**ACME-GW** <img src=x onerror=alert(1)>');
    await settle(f);
    expect(el(f, '[data-testid=knowledge-size]').textContent).toContain('of 64 KB');
    el<HTMLButtonElement>(f, '[data-testid=knowledge-preview-tab]').click();
    await settle(f);
    const preview = el(f, '[data-testid=knowledge-preview]');
    expect(preview.querySelector('strong')?.textContent).toBe('ACME-GW');
    expect(preview.querySelector('img')).toBeNull();
    expect(preview.textContent).toContain('<img src=x onerror=alert(1)>');
    save.click();
    await settle(f);
    expect(ref.close).toHaveBeenCalledWith(true);
    expect(at((await backend.listKnowledge()).project, 0)).toMatchObject({ name: 'Glossary', scope: 'project' });
  });

  it('edits an existing file, refuses too large text and stays open on a refusal', async () => {
    const backend = provide();
    const info = await backend.createKnowledge('global', 'Runbook', 'Restart the gateway.');
    const ref = { close: vi.fn(), closed: new Promise(() => undefined) };
    TestBed.configureTestingModule({ providers: [{ provide: DIALOG_REF, useValue: ref }] });
    const f = TestBed.createComponent(KnowledgeEditor);
    f.componentRef.setInput('file', info);
    f.componentRef.setInput('maxBytes', 30);
    await settle(f);
    expect(el<HTMLTextAreaElement>(f, '[data-testid=knowledge-text]').value).toBe('Restart the gateway.');
    expect(el<HTMLInputElement>(f, '[data-testid=knowledge-name]').value).toBe('Runbook');
    type(f, '[data-testid=knowledge-text]', 'x'.repeat(31));
    await settle(f);
    expect(el<HTMLButtonElement>(f, '[data-testid=knowledge-save]').disabled).toBe(true);
    expect(el(f, '[data-testid=knowledge-size]').classList).toContain('over');
    type(f, '[data-testid=knowledge-text]', 'Restart the gateway twice.');
    await settle(f);
    vi.spyOn(backend, 'updateKnowledge').mockRejectedValueOnce(new AiError('io', 'cannot save the knowledge file'));
    el<HTMLButtonElement>(f, '[data-testid=knowledge-save]').click();
    await settle(f);
    expect(ref.close).not.toHaveBeenCalled();
    expect(el(f, '[data-testid=knowledge-editor-error]').textContent).toContain('cannot save');
    el<HTMLButtonElement>(f, '[data-testid=knowledge-save]').click();
    await settle(f);
    expect(ref.close).toHaveBeenCalledWith(true);
    expect(await backend.getKnowledge('global', info.id)).toMatchObject({ text: 'Restart the gateway twice.' });
  });

  it('shows why a file could not be read', async () => {
    const { f, backend } = open();
    vi.spyOn(backend, 'getKnowledge').mockRejectedValueOnce(new AiError('notFound', 'this knowledge file no longer exists'));
    f.componentRef.setInput('file', { id: 'k9', scope: 'global', name: 'Gone', enabled: true, bytes: 1 });
    await settle(f);
    expect(text(f)).toContain('no longer exists');
    expect(el<HTMLButtonElement>(f, '[data-testid=knowledge-save]').disabled).toBe(true);
  });
});
