import { readFileSync, writeFileSync } from 'node:fs';
import { DIALOG_BACKEND_COMMANDS, DIALOG_BASE_COMMANDS, DIALOG_SYNC_COMMANDS } from './dialog-def';
import { DIALOG_DEFS } from './dialog-registry';
import { contractFile, DialogDef, KIT_BASE_COMMANDS } from './kit';

/** The checked-in contract file that Rust's guard reads (`app/src-tauri/dialog-contracts.json`). */
const FILE = 'src-tauri/dialog-contracts.json';

function generated(): string {
  const base = [...KIT_BASE_COMMANDS, ...DIALOG_BASE_COMMANDS, ...DIALOG_SYNC_COMMANDS];
  return `${JSON.stringify(contractFile(DIALOG_DEFS as unknown as readonly DialogDef[], base, DIALOG_BACKEND_COMMANDS), null, 2)}\n`;
}

/**
 * The registry is the source of every dialog's contract; Rust reads a generated copy, the way `generated_types.rs`
 * mirrors `api.types.ts`. When this fails, regenerate with `DEBRIEF_WRITE_CONTRACTS=1 npx ng test --watch=false
 * --include=src/app/dialog-windows/dialog-contracts.spec.ts` and review the diff: a command added to a dialog is a
 * widening of what that window may do.
 */
describe('dialog contracts file', () => {
  it('is the registry, serialised', () => {
    const want = generated();
    if (process.env['DEBRIEF_WRITE_CONTRACTS'] === '1') writeFileSync(FILE, want);
    expect(readFileSync(FILE, 'utf8')).toBe(want);
  });

  it('lists every dialog once, with the kind-derived minimum size', () => {
    const file = JSON.parse(generated()) as { dialogs: Record<string, { kind: string; minSize: { width: number; height: number } | null; commands: string[] }> };
    expect(Object.keys(file.dialogs).sort()).toEqual(DIALOG_DEFS.map((d) => d.id).sort());
    for (const d of DIALOG_DEFS) {
      const c = file.dialogs[d.id]!;
      expect(c.kind).toBe(d.kind);
      if (d.kind === 'viewer') expect(c.minSize, d.id).toEqual({ width: 680, height: 560 });
    }
  });
});
