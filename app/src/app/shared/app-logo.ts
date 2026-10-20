import { ChangeDetectionStrategy, Component, input } from '@angular/core';

let nextId = 0;

/**
 * IMT's app icon, inline (the CSP allows no remote images). The geometry is the small-size master
 * `src-tauri/icons-src/imt-icon-small.svg`, which reads best at title-bar and dialog sizes; keep them in step.
 * Decorative (hidden from assistive tech) unless `label` is set.
 */
@Component({
  selector: 'imt-app-logo',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<svg
    viewBox="0 0 1024 1024"
    [attr.width]="size()"
    [attr.height]="size()"
    [attr.aria-hidden]="label() ? null : 'true'"
    [attr.aria-label]="label() || null"
    [attr.role]="label() ? 'img' : null"
  >
    <defs>
      <linearGradient [attr.id]="gradientId" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0" stop-color="#4c8dff" />
        <stop offset="1" stop-color="#2a63d4" />
      </linearGradient>
    </defs>
    <rect x="16" y="16" width="992" height="992" rx="224" [attr.fill]="'url(#' + gradientId + ')'" stroke="#173f94" stroke-width="32" />
    <path d="M256 256H384M256 768H448" fill="none" stroke="#fff" stroke-opacity="0.7" stroke-width="128" stroke-linecap="round" />
    <path d="M256 512H448L544 352L640 672L704 512H768" fill="none" stroke="#fff" stroke-width="112" stroke-linecap="round" stroke-linejoin="round" />
    <circle cx="768" cy="256" r="88" fill="#ffc24b" />
  </svg>`,
  styles: `
    :host {
      display: inline-flex;
      flex: none;
    }
    svg {
      display: block;
    }
  `,
})
export class AppLogo {
  readonly size = input(20);
  /** Accessible name; when empty the logo is decorative. */
  readonly label = input('');
  /** Unique per instance, so several logos on one page don't share (or lose) a gradient. */
  protected readonly gradientId = `imt-logo-${nextId++}`;
}
