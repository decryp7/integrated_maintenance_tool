import AxeBuilder from '@axe-core/playwright';
import { expect, Page, TestInfo } from '@playwright/test';

/** The WCAG levels every axe check covers: 2.0, 2.1 and 2.2, A and AA. */
export const WCAG_TAGS = ['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'];

/** Runs axe against WCAG 2.2 A/AA and fails on any serious or critical violation. */
export async function expectNoSeriousViolations(page: Page, testInfo: TestInfo, label: string, include?: string): Promise<void> {
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
