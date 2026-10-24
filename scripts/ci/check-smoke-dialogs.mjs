// Real-app smoke specs never look for a registered dialog in the page (ADR 0050/0054): in the desktop app every
// registered dialog is its own OS window, so `inPageDialog` or a `role=dialog` selector finds nothing and the spec
// hangs behind the main window's modal scrim. Specs use `withDialogWindow` / `dismissDialogWindow` (support/app.ts).
import { readdirSync, readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const FORBIDDEN = [/\binPageDialog\b/, /role\s*=\s*['"]?dialog/];

/** The offending lines of one spec's source: `[{ line, text }]`. */
export function violations(source) {
  return source
    .split('\n')
    .map((text, i) => ({ line: i + 1, text }))
    .filter(({ text }) => !text.trim().startsWith('//') && FORBIDDEN.some((re) => re.test(text)));
}

export function main(specsDir, log = console) {
  let bad = 0;
  for (const f of readdirSync(specsDir).filter((n) => n.endsWith('.e2e.ts'))) {
    for (const v of violations(readFileSync(join(specsDir, f), 'utf8'))) {
      log.error(`${f}:${v.line}: a registered dialog is an OS window; use withDialogWindow/dismissDialogWindow (${v.text.trim()})`);
      bad++;
    }
  }
  return bad ? 1 : 0;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  process.exit(main(join(dirname(fileURLToPath(import.meta.url)), '../../app/e2e-tauri/specs')));
}
