import { DebriefActivityItem } from '../extensions/tokens';
import { AiStatusStore } from './ai-status.store';

/** The AI assistant button (opens the chat `view`): only once a model is set up; setting up stays in View ▸ AI. */
export function aiActivityItems(status: InstanceType<typeof AiStatusStore>, view: string): DebriefActivityItem[] {
  return [{ id: 'ai', label: $localize`:@@ai.activity.chat:AI assistant`, icon: 'ai', order: 700, when: () => status.hasModel(), run: { view } }];
}
