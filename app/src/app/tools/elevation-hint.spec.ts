import { Component, signal } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { all, buttonByText, settle, text } from '../../testing/helpers';
import { LifecycleService } from '../lifecycle/lifecycle.service';
import { MockToolsOptions } from './mock/mock-tools-backend';
import { mockTools, provideToolsTesting } from './mock/test-support';
import { ToolError } from './elevation-hint';

@Component({
  imports: [ToolError],
  template: `<imt-tool-error [message]="msg()" [retry]="retry()" (dismissed)="dismissed = dismissed + 1" (retried)="retried = retried + 1" />`,
})
class Host {
  msg = signal<string | null>(null);
  retry = signal(false);
  dismissed = 0;
  retried = 0;
}

async function setup(options: Partial<MockToolsOptions> = {}) {
  TestBed.configureTestingModule({ providers: provideToolsTesting(options) });
  await TestBed.inject(LifecycleService).init();
  const fixture = TestBed.createComponent(Host);
  await settle(fixture);
  return fixture;
}

describe('ToolError', () => {
  it('renders nothing without a message', async () => {
    const f = await setup();
    expect(all(f, '.err')).toHaveLength(0);
  });

  it('shows plain errors with dismiss and optional retry', async () => {
    const f = await setup();
    f.componentInstance.msg.set('disk full');
    f.componentInstance.retry.set(true);
    await settle(f);
    expect(text(f)).toContain('disk full');
    expect(text(f)).not.toContain('administrator');
    buttonByText(f, 'Retry').click();
    buttonByText(f, 'Dismiss').click();
    expect(f.componentInstance.retried).toBe(1);
    expect(f.componentInstance.dismissed).toBe(1);
  });

  it('offers a restart as administrator for access errors, where possible', async () => {
    const f = await setup();
    f.componentInstance.msg.set('Access is denied');
    await settle(f);
    buttonByText(f, 'Restart as administrator').click();
    await settle(f);
    expect(mockTools().calls.some((c) => c.startsWith('restartElevated'))).toBe(true);
  });

  it('tells the user to run as administrator where a restart is not possible, and nothing when already elevated', async () => {
    const mac = await setup({ platform: 'macos' });
    mac.componentInstance.msg.set('Access is denied');
    await settle(mac);
    expect(text(mac)).toContain('Run IMT as administrator');
    TestBed.resetTestingModule();
    const admin = await setup({ elevated: true });
    admin.componentInstance.msg.set('Access is denied');
    await settle(admin);
    expect(text(admin)).not.toContain('administrator and try again');
    expect(all(admin, 'button').map((b) => b.textContent?.trim())).toEqual(['Dismiss']);
  });
});
