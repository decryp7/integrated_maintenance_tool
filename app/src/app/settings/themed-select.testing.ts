import { ComponentFixture } from '@angular/core/testing';

/** Test helper: opens the `imt-select` whose trigger matches `trigger` and clicks the option whose text contains `label`. */
export async function pick(fixture: ComponentFixture<unknown>, trigger: string, label: string): Promise<void> {
  const button = fixture.nativeElement.querySelector(trigger) as HTMLButtonElement;
  button.click();
  fixture.detectChanges();
  await fixture.whenStable();
  const option = [...document.querySelectorAll<HTMLElement>('.cdk-overlay-container [role=option]')].find((o) => o.textContent?.includes(label));
  if (!option) throw new Error(`No option "${label}"`);
  option.click();
  fixture.detectChanges();
  await fixture.whenStable();
}
