import { afterNextRender, ChangeDetectionStrategy, Component, ElementRef, inject, OnDestroy } from '@angular/core';
import { DockService } from './dock.service';

/** Hosts the dockview instance managed by `DockService`. */
@Component({
  selector: 'imt-dock-host',
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: '',
  styles: `
    :host { display: block; flex: 1; min-height: 0; min-width: 0; position: relative; }
  `,
})
export class DockHost implements OnDestroy {
  private readonly dock = inject(DockService);
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);

  constructor() {
    afterNextRender(() => this.dock.attach(this.host.nativeElement));
  }

  ngOnDestroy(): void {
    this.dock.detach();
  }
}
