import { TestBed } from '@angular/core/testing';
import { el, FakeDock, provideFakeDock, provideTinyBackend, settle } from '../../testing/helpers';
import { LanguageService } from '../i18n/language.service';
import { LanguageChoice } from '../i18n/locale';
import { GeneralSettings } from './general-settings';

describe('GeneralSettings language', () => {
  async function setup(pending = false) {
    const chosen: LanguageChoice[] = [];
    let reloads = 0;
    const language = {
      choice: () => 'system' as LanguageChoice,
      reloadPending: () => pending,
      choose: async (c: LanguageChoice) => void chosen.push(c),
      reload: () => void reloads++,
    };
    TestBed.configureTestingModule({ providers: [provideTinyBackend(), provideFakeDock(new FakeDock()), { provide: LanguageService, useValue: language }] });
    const fixture = TestBed.createComponent(GeneralSettings);
    await settle(fixture);
    return { fixture, chosen, reloads: () => reloads };
  }

  it('lists System default, English (as an endonym) and Pseudo, and saves the choice', async () => {
    const { fixture, chosen } = await setup();
    const select = el<HTMLSelectElement>(fixture, '[data-testid=language-select]');
    expect([...select.options].map((o) => o.textContent?.trim())).toEqual(['System default', 'English', 'Pseudo']);
    expect(select.value).toBe('system');
    expect(select.options[1]?.getAttribute('lang')).toBe('en');
    select.value = 'pseudo';
    select.dispatchEvent(new Event('change'));
    expect(chosen).toEqual(['pseudo']);
    expect(fixture.nativeElement.querySelector('.pending')).toBeNull();
  });

  it('says a restart is pending and offers it', async () => {
    const { fixture, reloads } = await setup(true);
    expect(el(fixture, '.pending').textContent).toContain('The new language is used after IMT restarts.');
    el<HTMLButtonElement>(fixture, '.pending button').click();
    expect(reloads()).toBe(1);
  });
});
