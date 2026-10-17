import { ChangeDetectionStrategy, Component, computed, input } from '@angular/core';

/** Stroke icons drawn on a 24×24 grid. */
const PATHS: Record<string, string> = {
  files: 'M4 5h6l2 2h8v12H4z',
  search: 'M10.5 4.5a6 6 0 1 0 0 12a6 6 0 1 0 0-12M15 15l5 5',
  timeline: 'M3 7h10M7 12h14M3 17h8',
  metrics: 'M3 18l6-7l4 3l8-9',
  profile: 'M5 4h10l4 4v12H5zM9 11h6M9 15h6',
  settings: 'M12 9a3 3 0 1 0 0 6a3 3 0 1 0 0-6M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M5.6 18.4l2.1-2.1M16.3 7.7l2.1-2.1',
  plus: 'M12 5v14M5 12h14',
  minus: 'M5 12h14',
  close: 'M6 6l12 12M18 6L6 18',
  refresh: 'M19 12a7 7 0 1 1-2.1-5M19 4v4h-4',
  folder: 'M3 6h6l2 2h10v11H3z',
  file: 'M6 3h8l4 4v14H6zM14 3v4h4',
  chevronDown: 'M6 9l6 6l6-6',
  chevronRight: 'M9 6l6 6l-6 6',
  group: 'M8 12a3 3 0 1 0 0 .1M14 7h7M14 12h7M14 17h7',
  trash: 'M5 7h14M10 7V4h4v3M7 7l1 13h8l1-13',
  copy: 'M9 9h11v11H9zM5 15V4h11',
  columns: 'M4 5h16v14H4zM10 5v14M16 5v14',
  sortAsc: 'M8 18V6M5 9l3-3l3 3M14 8h6M14 12h4M14 16h2',
  sortDesc: 'M8 6v12M5 15l3 3l3-3M14 8h2M14 12h4M14 16h6',
  sun: 'M12 8a4 4 0 1 0 0 8a4 4 0 1 0 0-8M12 2v2M12 20v2M2 12h2M20 12h2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4',
  moon: 'M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z',
  check: 'M5 12.5l4.5 4.5L19 7',
  alert: 'M12 4l9 16H3zM12 10v4M12 17v.5',
  zoomOut: 'M10.5 4.5a6 6 0 1 0 0 12a6 6 0 1 0 0-12M15 15l5 5M8 10.5h5',
  drag: 'M9 6h.01M15 6h.01M9 12h.01M15 12h.01M9 18h.01M15 18h.01',
  panel: 'M4 5h16v14H4zM15 5v14',
  chart: 'M4 20V4M4 20h16M8 16v-5M12 16V8M16 16v-3',
  puzzle: 'M5 9h4a2 2 0 1 1 4 0h4v4a2 2 0 1 1 0 4v3H5z',
};

@Component({
  selector: 'imt-icon',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<svg [attr.width]="size()" [attr.height]="size()" viewBox="0 0 24 24" aria-hidden="true">
    <path [attr.d]="d()" fill="none" stroke="currentColor" [attr.stroke-width]="stroke()" stroke-linecap="round" stroke-linejoin="round" />
  </svg>`,
  styles: `:host { display: inline-flex; flex: none; }`,
})
export class Icon {
  readonly name = input.required<string>();
  readonly size = input(16);
  readonly stroke = input(1.7);
  protected readonly d = computed(() => PATHS[this.name()] ?? '');
}
