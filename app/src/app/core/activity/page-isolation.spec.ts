import { activeViewTitle } from './runtime';

// Spec files share one page (the Angular builder runs Vitest with `isolate: false`). Elements a spec left in the body
// (a dock with an active tab, for example) were read by the next spec: an uncaught error was logged with
// `View: "Inspector"` in the logger spec, depending on which files a worker ran together. test-setup.ts empties the
// body before every test; the two tests run in order and prove it.
describe('the page between tests', () => {
  it('a test may leave a dock with an active tab in the page', () => {
    const tab = document.createElement('div');
    tab.className = 'dv-tab dv-active-tab';
    tab.textContent = 'Inspector';
    document.body.appendChild(tab);
    expect(activeViewTitle()).toBe('Inspector');
  });

  it('the next test starts with an empty page', () => {
    expect(document.body.childElementCount).toBe(0);
    expect(activeViewTitle()).toBeUndefined();
  });
});
