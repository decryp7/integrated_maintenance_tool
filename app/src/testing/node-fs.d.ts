/** The few Node APIs the activity coverage guard uses to scan the sources (Vitest runs in Node). Specs only. */
declare module 'node:fs' {
  export function readFileSync(path: string, encoding: 'utf8'): string;
  export function readdirSync(path: string, options: { recursive: true }): string[];
}
declare const process: { cwd(): string };
