/**
 * When a recent item was opened, for its list: the time today ("14:05"), otherwise the date ("14 Oct 2026").
 * A pure function of the instant and the clock; the viewer's locale and time zone apply.
 */
export function formatOpenedAt(openedAt: number, now: number = Date.now(), locale?: string): string {
  if (!Number.isFinite(openedAt) || openedAt <= 0) return '';
  const when = new Date(openedAt);
  const today = new Date(now);
  const sameDay = when.getFullYear() === today.getFullYear() && when.getMonth() === today.getMonth() && when.getDate() === today.getDate();
  return sameDay
    ? new Intl.DateTimeFormat(locale, { hour: '2-digit', minute: '2-digit' }).format(when)
    : new Intl.DateTimeFormat(locale, { day: 'numeric', month: 'short', year: 'numeric' }).format(when);
}
