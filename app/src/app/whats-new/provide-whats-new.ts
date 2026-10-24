import { EnvironmentProviders, makeEnvironmentProviders } from '@angular/core';
import { provideDebriefPlugin } from '../extensions/provide';
import { DebriefView } from '../extensions/tokens';
import { WHATS_NEW_VIEW } from './whats-new.service';

export { WHATS_NEW_NOTICE, WHATS_NEW_SEEN_KEY, WHATS_NEW_VIEW, WhatsNewService } from './whats-new.service';

/**
 * What's new is a document of the dock (ADR 0056 declarations): a full-height reading column like Home, below Home in
 * priority so it is the one tabbed when the window is short of room. Reached from Help ▸ What's new and the palette,
 * not listed in the View menu.
 */
export const WHATS_NEW_VIEWS: DebriefView[] = [
  {
    id: WHATS_NEW_VIEW,
    title: $localize`:@@whatsNew.title:What's new`,
    category: 'core',
    icon: 'note',
    defaultPosition: 'center',
    layout: { area: 'document', min: { width: 420, height: 300 }, pref: { width: 640, height: 600 }, priority: 45 },
    fullHeight: true,
    menu: 'palette',
    component: () => import('./whats-new-view').then((m) => m.WhatsNewView),
  },
];

/** One line in `app.config.ts`: the What's new document (the Help command and the start-up check live elsewhere). */
export function provideWhatsNew(): EnvironmentProviders {
  return makeEnvironmentProviders([provideDebriefPlugin({ views: WHATS_NEW_VIEWS })]);
}
