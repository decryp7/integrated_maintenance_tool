import { Notice, NoticeSource } from './notifications.types';

export function sourceLabel(source: NoticeSource): string {
  switch (source) {
    case 'job':
      return $localize`:@@notifications.source.job:Jobs`;
    case 'liveAlert':
      return $localize`:@@notifications.source.liveAlert:Live alerts`;
    case 'import':
      return $localize`:@@notifications.source.import:Imports`;
    case 'plugin':
      return $localize`:@@notifications.source.plugin:Plugins`;
    case 'ai':
      return $localize`:@@notifications.source.ai:AI`;
    case 'update':
      return $localize`:@@notifications.source.update:Updates`;
    case 'session':
      return $localize`:@@notifications.source.session:Session`;
    case 'performance':
      return $localize`:@@notifications.source.performance:Performance`;
  }
}

/** "Error spike on WEB02 (×4)" when it happened more than once. */
export function titleWithCount(n: Pick<Notice, 'title' | 'count'>): string {
  return n.count > 1 ? $localize`:@@notifications.titleCount:${n.title}:title: (×${n.count}:count:)` : n.title;
}
