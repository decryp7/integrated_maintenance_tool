import { AiEngineStatus } from './ai.types';

/** The pairing code as the user compares it in Pitwall: six digits in two groups ("123 456"); anything else as it is. */
export function groupCode(code: string): string {
  return /^\d{6}$/.test(code) ? `${code.slice(0, 3)} ${code.slice(3)}` : code;
}

/** One plain sentence about the AI engine and Pitwall, for Settings → AI. */
export function engineSentence(e: AiEngineStatus): string {
  const p = e.pitwall;
  if (e.active === 'pitwall') {
    const version = p.version ? ` ${p.version}` : '';
    return p.model
      ? $localize`:@@ai.engine.pitwallModel:Engine: Pitwall${version}:version: · model ${p.model}:model:`
      : $localize`:@@ai.engine.pitwall:Engine: Pitwall${version}:version:`;
  }
  if (e.active === 'builtIn') {
    const base = $localize`:@@ai.engine.builtIn:Engine: built-in`;
    const hint = pitwallHint(e);
    return hint ? `${base}. ${hint}` : base;
  }
  return $localize`:@@ai.engine.none:Engine: none. Pitwall is chosen but cannot be used right now.`;
}

/** What to do about Pitwall when it is on this computer but not in use; null when there is nothing to say. */
function pitwallHint(e: AiEngineStatus): string | null {
  const p = e.pitwall;
  switch (p.found) {
    case 'running':
      return p.paired
        ? null
        : $localize`:@@ai.engine.running:Pitwall is running on this computer. Connect Debrief to it to share its models.`;
    case 'installed':
      return $localize`:@@ai.engine.installed:Pitwall is installed but not running. Open Pitwall to use it.`;
    case 'incompatible': {
      const version = p.version ?? '';
      return $localize`:@@ai.engine.incompatible:Pitwall ${version}:version: speaks a newer connection version; update Debrief.`;
    }
    case 'absent':
      return null;
  }
}

/** The sentence plus what Pitwall reports as the problem, if any. */
export function engineLines(e: AiEngineStatus): string[] {
  const lines = [engineSentence(e)];
  const hint = e.active === null ? pitwallHint(e) : null;
  if (hint) lines.push(hint);
  if (e.pitwall.problem) lines.push(e.pitwall.problem);
  return lines;
}
