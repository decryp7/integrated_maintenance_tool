import { expect, Page, test } from '@playwright/test';
import { openApp } from './app';
import { chooseMenu } from './menu-support';
import { openAppWith } from './mock-url';

// Third-party plugin packages in the Plugins panel, against the mock backend: in the browser build "Install from
// file…" picks `/demo/shared.imtplugin`, a package signed by a trusted publisher; `?mock-plugin-package=unsigned`
// makes every package unsigned, which IMT refuses outside developer mode.

async function openPlugins(page: Page) {
  await chooseMenu(page, 'View', 'Plugins', 'Manage plugins');
  const panel = page.locator('imt-plugins-view');
  await expect(panel).toBeVisible();
  return panel;
}

test('installs a signed package after reviewing its publisher and permissions', async ({ page }) => {
  await openApp(page);
  const panel = await openPlugins(page);
  const installed = panel.getByRole('list', { name: 'Installed from packages' }).getByRole('listitem');
  await expect(installed).toHaveCount(1);

  await panel.getByRole('button', { name: 'Install from file…' }).click();
  const review = page.getByRole('dialog', { name: 'Install plugin' });
  await expect(review.getByTestId('package-name')).toHaveText('Web access log enricher');
  await expect(review.getByTestId('package-publisher')).toHaveText('Example Software');
  await expect(review.getByTestId('package-signature')).toHaveText('Signed by Example Software Inc.');
  await expect(review.getByTestId('package-refusal')).toHaveCount(0);
  // Every requested permission is offered with the publisher's reason; withhold one of them.
  const runPrograms = review.getByRole('checkbox', { name: /Run programs/ });
  const network = review.getByRole('checkbox', { name: /Network access/ });
  await expect(runPrograms).toBeChecked();
  await expect(network).toBeChecked();
  await expect(review).toContainText("Publisher's reason: To run the GeoIP database updater.");
  await runPrograms.uncheck();
  await review.getByRole('button', { name: 'Install', exact: true }).click();
  await expect(review).toBeHidden();

  await expect(installed).toHaveCount(2);
  const added = panel.locator('li.package[data-id="net.example.web-enricher"]');
  await expect(added).toContainText('Web access log enricher');
  await expect(added.getByTestId('package-signature')).toHaveText('Signed by Example Software Inc.');
  await expect(added.getByTestId('package-state')).toHaveText('pending restart');
  await expect(added.getByTestId('package-granted')).toContainText('Network access');
  await expect(added.getByTestId('package-granted')).not.toContainText('Run programs');
  await expect(panel.getByTestId('packages-restart')).toContainText('Changes apply after a restart.');
});

test('refuses an unsigned package and installs nothing', async ({ page }) => {
  await openAppWith(page, '?mock-plugin-package=unsigned');
  const panel = await openPlugins(page);
  const installed = panel.getByRole('list', { name: 'Installed from packages' }).getByRole('listitem');
  await expect(installed).toHaveCount(1);

  await panel.getByRole('button', { name: 'Install from file…' }).click();
  const review = page.getByRole('dialog', { name: 'Install plugin' });
  await expect(review.getByTestId('package-name')).toHaveText('Homebrew parser');
  await expect(review.getByTestId('package-signature')).toHaveText('Not signed');
  await expect(review.getByTestId('package-refusal')).toContainText('The package is not signed, so IMT cannot tell who made it.');
  await expect(review.getByTestId('package-refusal')).toContainText('Settings → Plugin trust');
  // Nothing can be granted or installed.
  await expect(review.getByRole('checkbox', { name: /Network access/ })).toBeDisabled();
  await expect(review.getByRole('checkbox', { name: /Network access/ })).not.toBeChecked();
  await expect(review.getByRole('button', { name: 'Install', exact: true })).toBeDisabled();
  await review.getByRole('button', { name: 'Cancel' }).click();
  await expect(review).toBeHidden();
  await expect(installed).toHaveCount(1);
  await expect(panel.locator('li.package[data-id="org.example.homebrew"]')).toHaveCount(0);
});
