/** Splits a value after each `.`, `/` and `\` so a narrow column can wrap it between segments instead of mid-word. Pure. */
export function breakAfterSeparators(value: string): string[] {
  return value.split(/(?<=[./\\])/);
}
