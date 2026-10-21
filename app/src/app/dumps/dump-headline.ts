import { DumpHeadline } from '../core/api.types';
import { plural } from '../i18n/locale';
import { formatCount } from '../shared/format/units';
import { formatBytes, formatDelta } from './dumps-shared';

/**
 * A document's headline in words: `ACCESS_VIOLATION in ntdll.dll!RtlFreeHeap+0x1d`, `4 threads, no exception`,
 * `412 allocation stacks, 38 MB`, `+2 MB across 3 allocation stacks`. The engine sends the values; sizes and
 * counts are formatted here with the shared units rules.
 */
export function headlineText(h: DumpHeadline): string {
  switch (h.type) {
    case 'exception':
      return h.location ? $localize`:@@dumps.headline.exceptionAt:${h.name}:name: in ${h.location}:location:` : h.name;
    case 'noException': {
      const n = formatCount(h.threads);
      return plural(h.threads, {
        one: $localize`:@@dumps.headline.noException.one:${n}:count: thread, no exception`,
        other: $localize`:@@dumps.headline.noException.other:${n}:count: threads, no exception`,
      });
    }
    case 'umdhSnapshot': {
      const n = formatCount(h.stacks);
      const size = formatBytes(h.bytes);
      return plural(h.stacks, {
        one: $localize`:@@dumps.headline.snapshot.one:${n}:count: allocation stack, ${size}:size:`,
        other: $localize`:@@dumps.headline.snapshot.other:${n}:count: allocation stacks, ${size}:size:`,
      });
    }
    case 'umdhDiff': {
      const n = formatCount(h.stacks);
      const delta = formatDelta(h.deltaBytes);
      return plural(h.stacks, {
        one: $localize`:@@dumps.headline.diff.one:${delta}:delta: across ${n}:count: allocation stack`,
        other: $localize`:@@dumps.headline.diff.other:${delta}:delta: across ${n}:count: allocation stacks`,
      });
    }
  }
}
