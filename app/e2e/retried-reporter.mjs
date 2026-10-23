// Playwright reporter: lists the tests that passed only after a retry (flaky) in the job summary, so a retry never
// hides a flake (Paddock rule T06). Writes Markdown to $GITHUB_STEP_SUMMARY when set, always prints the list, and
// writes playwright-retried.md next to the report. It never changes the exit code: fixing the flake is a test task.
import { appendFileSync, writeFileSync } from 'node:fs';

/** Markdown for the flaky tests (empty string when there are none). */
export function render(flaky) {
  if (!flaky.length) return '';
  const rows = flaky.map((f) => `| ${f.project} | ${f.file} | ${f.title} | ${f.retries} |`).join('\n');
  return `### Flaky tests (passed only after a retry)\n\n| Project | File | Test | Retries |\n| --- | --- | --- | --- |\n${rows}\n`;
}

export default class RetriedReporter {
  flaky = [];
  onTestEnd(test, result) {
    // The final attempt passed but an earlier one did not: flaky.
    if (result.status === 'passed' && result.retry > 0) {
      this.flaky.push({
        project: test.parent.project()?.name ?? '',
        file: test.location.file.replace(/^.*[\\/]e2e[\\/]/, ''),
        title: test.titlePath().slice(3).join(' > ').replaceAll('|', '\\|'),
        retries: result.retry,
      });
    }
  }
  onEnd() {
    const md = render(this.flaky);
    if (!md) return;
    console.log(`\n${md}`);
    try {
      writeFileSync('playwright-retried.md', md);
      if (process.env['GITHUB_STEP_SUMMARY']) appendFileSync(process.env['GITHUB_STEP_SUMMARY'], `${md}\n`);
    } catch {
      // The summary is a convenience; the console list above is the record.
    }
  }
  printsToStdio() {
    return false;
  }
}
