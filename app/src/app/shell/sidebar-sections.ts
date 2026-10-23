import { Injectable, signal } from '@angular/core';

export const FIELDS_OPEN_KEY = 'debrief.sidebar.fieldsOpen.v1';

function readOpen(): boolean {
  try {
    return localStorage.getItem(FIELDS_OPEN_KEY) === 'true';
  } catch {
    return false;
  }
}

/** Which sidebar sections are expanded: the Fields section starts collapsed and remembers the user's choice. */
@Injectable({ providedIn: 'root' })
export class SidebarSections {
  private readonly open = signal(readOpen());
  readonly fieldsOpen = this.open.asReadonly();

  setFieldsOpen(open: boolean): void {
    this.open.set(open);
    try {
      localStorage.setItem(FIELDS_OPEN_KEY, String(open));
    } catch {
      /* storage unavailable: the choice lasts for this session only */
    }
  }
}
