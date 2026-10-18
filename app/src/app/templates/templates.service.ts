import { inject, Injectable } from '@angular/core';
import { DialogService } from '../ui/dialog.service';

/** A file to preview the templates against (an opened or unrecognised file of the workspace). */
export interface TemplateTarget {
  path: string;
  name: string;
}

/** Opens the templates gallery ("New from template", "Start from a template" for an unrecognised file). */
@Injectable({ providedIn: 'root' })
export class TemplatesService {
  private readonly dialogs = inject(DialogService);

  /** Resolves true when a template was applied. */
  async open(target: TemplateTarget | null = null): Promise<boolean> {
    // Loaded on demand: the gallery is not part of the initial bundle.
    const { TemplatesGallery } = await import('./templates-gallery');
    const ref = this.dialogs.open<boolean>(TemplatesGallery, { title: $localize`:@@templates.service.title:New profile from a template`, width: 900, inputs: { path: target?.path ?? null, name: target?.name ?? null } });
    return (await ref.closed) === true;
  }
}
