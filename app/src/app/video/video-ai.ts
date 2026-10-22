import { inject, Injectable, Injector } from '@angular/core';
import { AI_IMAGE_VIEW } from '../ai/ai-launcher';
import { FrameRef } from '../core/generated/video';
import { COMMAND_CONTEXT } from '../extensions/command-registry';
import { VideoBackend } from './video-backend';

/** What the AI can do with a video frame: the image use cases of ADR 0025, on the clean frame the engine opened. */
export type FrameAiUse = 'explain' | 'related';

/**
 * "Explain with AI" and "Find related logs" for a frame. The engine puts the clean frame into the window's image cache
 * with its aligned time as evidence; from there the existing image store runs the same AI jobs as for any image, and
 * the image view shows the answer. Resolves once the image is shown and the job is started (the job goes on in Jobs).
 */
@Injectable({ providedIn: 'root' })
export class VideoAi {
  private readonly injector = inject(Injector);
  private readonly backend = inject(VideoBackend);

  async run(use: FrameAiUse, frame: FrameRef): Promise<void> {
    const image = await this.backend.frameToImage(frame);
    const { AiImageStore } = await import('../ai/vision/ai-image.store');
    const images = this.injector.get(AiImageStore);
    images.adopt(image);
    this.injector.get(COMMAND_CONTEXT).showView(AI_IMAGE_VIEW);
    void (use === 'explain' ? images.explain(image.id) : images.findRelated(image.id));
  }
}
