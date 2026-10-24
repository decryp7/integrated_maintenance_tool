import { DialogComponent, DIALOG_BACKEND_COMMANDS, DIALOG_BASE_COMMANDS, DIALOG_BOOT_COMMANDS, DIALOG_SYNC_COMMANDS, dialogIdOf } from './dialog-def';
import { DIALOG_DEFS, dialogDef } from './dialog-registry';
import { DIALOG_KINDS, KIND_POLICY, KIT_BASE_COMMANDS, VIEWER_MIN, commandCovered, defineDialog, modalityOf, raiseToKindMinimum, resolveSize, type } from './kit';

const load = async () => class {};
const base = { width: 400, result: type<void>(), commands: [], load } as const;

describe('dialog registry', () => {
  it('has unique, well-formed ids and a kind for every dialog', () => {
    const ids = DIALOG_DEFS.map((d) => d.id);
    expect(new Set(ids).size).toBe(ids.length);
    expect(ids.length).toBe(51);
    for (const d of DIALOG_DEFS) {
      expect(d.id).toMatch(/^[a-z0-9][a-z0-9-]{0,63}$/);
      expect(DIALOG_KINDS, d.id).toContain(d.kind);
      expect(['modal', 'modeless']).toContain(d.modality);
      expect(d.width).toBeGreaterThanOrEqual(320);
      expect(d.commands.length, d.id).toBeGreaterThanOrEqual(0);
      expect(new Set(d.commands).size, `${d.id} lists a command twice`).toBe(d.commands.length);
      const implied = [...KIT_BASE_COMMANDS, ...DIALOG_BASE_COMMANDS, ...DIALOG_BOOT_COMMANDS, ...DIALOG_SYNC_COMMANDS, ...(d.backends ?? []).flatMap((b) => DIALOG_BACKEND_COMMANDS[b] ?? [])];
      expect(d.commands.filter((c) => commandCovered(implied, c)), `${d.id} lists a base command`).toEqual([]);
    }
    expect(dialogDef('nope')).toBeUndefined();
  });

  it('derives modality from the kind: decisions are modal, viewers and tools modeless, forms choose', () => {
    for (const d of DIALOG_DEFS) {
      expect(KIND_POLICY[d.kind].modalities, d.id).toContain(d.modality);
      if (d.kind === 'decision') expect(d.modality, d.id).toBe('modal');
      if (d.kind === 'viewer' || d.kind === 'tool') expect(d.modality, d.id).toBe('modeless');
    }
    expect(DIALOG_DEFS.filter((d) => d.kind === 'form').map((d) => d.modality)).toEqual(expect.arrayContaining(['modal', 'modeless']));
  });

  it('keeps the decided kinds (ADR 0054): result windows are viewers, incident-summary included, and Settings is a tool', () => {
    const kinds = (kind: string) => DIALOG_DEFS.filter((d) => d.kind === kind).map((d) => d.id).sort();
    expect(kinds('viewer')).toEqual(['about', 'ai-narrative', 'ai-proposal-result', 'ai-regex', 'cheat-sheet', 'incident-summary', 'updates', 'whats-new']);
    expect(kinds('tool')).toEqual(['ai-models', 'research', 'settings']);
    expect(kinds('decision')).toEqual(
      ['close-prompt', 'encoding', 'import-range', 'marker-override', 'open-ask', 'open-live', 'package-review', 'parser-import-confirm', 'plugin-permissions', 'recent-projects', 'save-layout', 'unsaved-changes'].sort(),
    );
    // The editors and tools that are used beside the grid stay modeless (they were made so on purpose in 1.0.2).
    for (const id of ['incident-summary', 'note-editor', 'knowledge-editor', 'parser-editor', 'format-wizard', 'search-web', 'report-export']) {
      expect(dialogDef(id)?.modality, id).toBe('modeless');
    }
    // Only a modal without a Cancel is a required choice.
    expect(DIALOG_DEFS.filter((d) => d.requiredChoice).map((d) => d.id)).toEqual(['close-prompt']);
  });

  it('gives viewers 680x560 at least, tools their declared size and fits only what names no height', () => {
    for (const d of DIALOG_DEFS) {
      if (d.kind === 'viewer') {
        expect(d.width, d.id).toBeGreaterThanOrEqual(VIEWER_MIN.width);
        expect(d.height, d.id).toBeGreaterThanOrEqual(VIEWER_MIN.height);
        expect(d.minWidth, d.id).toBeGreaterThanOrEqual(VIEWER_MIN.width);
        expect(d.minHeight, d.id).toBeGreaterThanOrEqual(VIEWER_MIN.height);
        expect(d.fit, `${d.id}: a viewer grows with its content`).toBe(true);
      }
      if (d.kind === 'tool') expect(d.fit, `${d.id}: a tool keeps its size`).toBe(false);
      if (d.kind === 'decision') expect(d.height, `${d.id}: a decision is fitted to its content`).toBeUndefined();
    }
    expect(dialogDef('ai-narrative')?.height).toBeGreaterThanOrEqual(560);
    expect(dialogDef('whats-new')?.width).toBe(680);
    expect(dialogDef('settings')?.height).toBeGreaterThanOrEqual(680);
  });

  it('loads every dialog, and each component declares the id it is registered under', async () => {
    for (const d of DIALOG_DEFS) {
      const component = await d.load();
      expect(dialogIdOf(component), d.id).toBe(d.id);
      expect(typeof component).toBe('function');
    }
  });

  it('reads an id only from a component that declares one', () => {
    class A {
      static readonly dialogId = 'a';
    }
    class B {}
    expect(dialogIdOf(A)).toBe('a');
    expect(dialogIdOf(B)).toBeUndefined();
    expect((A as unknown as DialogComponent).dialogCanClose).toBeUndefined();
  });
});

describe('defineDialog and the kind policy', () => {
  it('refuses a combination the kind forbids, once, when the dialog is defined', () => {
    expect(() => defineDialog({ ...base, id: 'x', kind: 'decision', modality: 'modeless' })).toThrow(/cannot be modeless/);
    expect(() => defineDialog({ ...base, id: 'x', kind: 'viewer', modality: 'modal' })).toThrow(/cannot be modal/);
    expect(() => defineDialog({ ...base, id: 'x', kind: 'form' })).toThrow(/must declare its modality/);
    expect(() => defineDialog({ ...base, id: 'x', kind: 'form', modality: 'modeless', requiredChoice: true })).toThrow(/required choice/);
    expect(() => defineDialog({ ...base, id: 'X y', kind: 'tool' })).toThrow(/malformed/);
    expect(() => defineDialog({ id: 'x', kind: 'decision', result: type<void>(), commands: [], load })).toThrow(/declares its width/);
  });

  it('applies the kind defaults: decisions fit, forms fit unless sized, viewers open at 680x560 and tools at 960x680', () => {
    expect(defineDialog({ ...base, id: 'd', kind: 'decision' })).toMatchObject({ modality: 'modal', height: undefined, fit: true });
    expect(defineDialog({ ...base, id: 'f', kind: 'form', modality: 'modal' })).toMatchObject({ fit: true });
    expect(defineDialog({ ...base, id: 'f', kind: 'form', modality: 'modeless', height: 500 })).toMatchObject({ height: 500, fit: false });
    expect(defineDialog({ id: 'v', kind: 'viewer', result: type<void>(), commands: [], load })).toMatchObject({ modality: 'modeless', width: 680, height: 560, minWidth: 680, minHeight: 560, fit: true });
    expect(defineDialog({ id: 'v', kind: 'viewer', width: 500, height: 300, result: type<void>(), commands: [], load })).toMatchObject({ width: 680, height: 560 });
    expect(defineDialog({ id: 'v', kind: 'viewer', width: 900, result: type<void>(), commands: [], load })).toMatchObject({ width: 900, height: 560 });
    expect(defineDialog({ id: 't', kind: 'tool', result: type<void>(), commands: [], load })).toMatchObject({ width: 960, height: 680, fit: false });
    expect(resolveSize('tool', { width: 700, minWidth: 500 })).toMatchObject({ width: 700, height: 680, minWidth: 500 });
    expect(modalityOf('tool', undefined)).toBe('modeless');
  });

  it('raises a remembered size below the kind minimum and leaves the others', () => {
    expect(raiseToKindMinimum('viewer', { x: 1, width: 400, height: 300 })).toEqual({ x: 1, width: 680, height: 560 });
    expect(raiseToKindMinimum('viewer', { width: 900, height: 700 })).toEqual({ width: 900, height: 700 });
    expect(raiseToKindMinimum('decision', { width: 300, height: 100 })).toEqual({ width: 300, height: 100 });
  });

  it('matches declared commands exactly or by prefix', () => {
    expect(commandCovered(['a_b', 'ui_*'], 'a_b')).toBe(true);
    expect(commandCovered(['a_b', 'ui_*'], 'ui_sync')).toBe(true);
    expect(commandCovered(['a_b', 'ui_*'], 'a_bc')).toBe(false);
  });
});
