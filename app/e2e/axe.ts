import AxeBuilder from '@axe-core/playwright';
import { expect, Page, TestInfo } from '@playwright/test';

/** The WCAG levels every axe check covers: 2.0, 2.1 and 2.2, A and AA. */
export const WCAG_TAGS = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'];

/** Runs axe against WCAG 2.2 A/AA and fails on any serious or critical violation. */
/** Waits until no CSS animation or transition is running: axe measures colours as rendered, so a panel sliding in or a
 *  scrim fading out mid-scan reads as low contrast (WebKit, being slower, used to hit this). */
export async function settleAnimations(page: Page): Promise<void> {
  // A panel opened by the last click starts its animation when the next frames render: checking before then sees
  // nothing running and scans a half-faded panel. Two frames let pending renders and style changes start theirs.
  await page.evaluate(() => new Promise<void>((done) => requestAnimationFrame(() => requestAnimationFrame(() => done()))));
  // Endless animations (spinners, a pulsing live dot) are ignored: they never settle. So are instant ones: with reduced
  // motion the motion tokens are 0.01ms, and WebKit can leave such a transition "running" without ever finishing it.
  await page.waitForFunction(
    () =>
      document.getAnimations().every((a) => {
        const timing = a.effect?.getTiming();
        return a.playState !== 'running' || timing?.iterations === Infinity || Number(timing?.duration ?? 0) <= 1;
      }),
    undefined,
    { timeout: 10_000 },
  );
}

export async function expectNoSeriousViolations(page: Page, testInfo: TestInfo, label: string, include?: string): Promise<void> {
  await settleAnimations(page);
  const builder = new AxeBuilder({ page }).withTags(WCAG_TAGS);
  const { violations } = await (include ? builder.include(include) : builder).analyze();
  const blocking = violations
    .filter((v) => v.impact === 'serious' || v.impact === 'critical')
    .map((v) => ({ id: v.id, impact: v.impact, help: v.help, count: v.nodes.length, nodes: v.nodes.slice(0, 8).map((n) => ({ target: n.target.join(" "), html: n.html.slice(0, 300), data: n.any[0]?.data, related: n.any[0]?.relatedNodes?.slice(0, 4).map((r) => r.target.join(' ')) })) }));
  if (blocking.length) {
    console.log(`AXE ${label} ${JSON.stringify(blocking)}`);
    await testInfo.attach(`axe-${label}.json`, { body: JSON.stringify(blocking, null, 2), contentType: 'application/json' });
  }
  expect(blocking, `axe violations (${label})`).toEqual([]);
}

/**
 * Waits until the dock layout has settled: every group's box is the same on two animation frames in a row, for several
 * checks in a row. The split between the Timeline and the Events panels is applied after the first paint (dockview
 * sizes its groups from the measured container), so a screenshot taken earlier differs from run to run.
 */
export async function settleDockLayout(page: Page): Promise<void> {
  await page.waitForFunction(
    () => {
      const w = window as unknown as { __dockBoxes?: string; __dockStable?: number };
      const boxes = () =>
        [...document.querySelectorAll('.dv-groupview')]
          .map((g) => {
            const r = g.getBoundingClientRect();
            return `${Math.round(r.x)},${Math.round(r.y)},${Math.round(r.width)},${Math.round(r.height)}`;
          })
          .join('|');
      const now = boxes();
      w.__dockStable = now && now === w.__dockBoxes ? (w.__dockStable ?? 0) + 1 : 0;
      w.__dockBoxes = now;
      return w.__dockStable >= 6;
    },
    undefined,
    { polling: 'raf', timeout: 10_000 },
  );
}
