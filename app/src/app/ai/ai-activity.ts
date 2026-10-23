import { DebriefActivityItem } from '../extensions/tokens';
import { AiStatusStore } from './ai-status.store';

/** The AI assistant button (opens the chat `view`) once a model is set up; before that, a button that starts the set-up. */
export function aiActivityItems(status: InstanceType<typeof AiStatusStore>, view: string): DebriefActivityItem[] {
  return [
    { id: 'ai', label: $localize`:@@ai.activity.chat:AI assistant`, icon: 'ai', order: 700, when: () => status.hasModel(), run: { view } },
    { id: 'ai-setup', label: $localize`:@@ai.activity.setup:Set up AI assistant`, icon: 'ai', order: 700, when: () => status.needsSetup(), run: { command: 'ai.setup' } },
  ];
}
