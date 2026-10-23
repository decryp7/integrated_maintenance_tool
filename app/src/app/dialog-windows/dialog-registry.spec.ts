import { DialogComponent, dialogIdOf } from './dialog-def';
import { DIALOG_DEFS, dialogDef } from './dialog-registry';

describe('dialog registry', () => {
  it('has unique, well-formed ids and a modality for every dialog', () => {
    const ids = DIALOG_DEFS.map((d) => d.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const d of DIALOG_DEFS) {
      expect(d.id).toMatch(/^[a-z0-9][a-z0-9-]{0,63}$/);
      expect(['modal', 'modeless']).toContain(d.modality);
      expect(d.width).toBeGreaterThanOrEqual(320);
    }
    expect(dialogDef('settings')?.modality).toBe('modeless');
    expect(dialogDef('ai-setup')?.modality).toBe('modal');
    expect(dialogDef('unsaved-changes')?.modality).toBe('modal');
    expect(dialogDef('nope')).toBeUndefined();
  });

  it('keeps the decided modality: the AI setup is a modal choice, the editors and result windows are tools', () => {
    for (const id of ['incident-summary', 'note-editor', 'knowledge-editor', 'parser-editor', 'format-wizard', 'search-web', 'report-export']) {
      expect(dialogDef(id)?.modality, id).toBe('modeless');
    }
    // Only a modal without a Cancel is a required choice.
    expect(DIALOG_DEFS.filter((d) => d.requiredChoice).map((d) => d.id)).toEqual(['close-prompt']);
    expect(DIALOG_DEFS.filter((d) => d.requiredChoice && d.modality !== 'modal')).toEqual([]);
  });

  it('gives dialogs that stream or load results a default height, so they never open fitted to a loading state', () => {
    for (const id of ['ai-narrative', 'ai-proposal-result', 'ai-query', 'incident-summary', 'research', 'whats-new']) {
      expect(dialogDef(id)?.height, id).toBeGreaterThanOrEqual(480);
    }
    expect(dialogDef('unsaved-changes')?.height).toBeUndefined();
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
