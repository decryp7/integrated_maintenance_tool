import { computed, DestroyRef, inject } from '@angular/core';
import { patchState, signalStore, withComputed, withMethods, withState } from '@ngrx/signals';
import { JobFollow } from '../../jobs/job-follow';
import { AiBackend, toAiError } from '../ai-backend';
import { AiStatusStore } from '../ai-status.store';
import { AiImage, InstalledModel, Narrative, RelatedOutcome } from '../ai.types';
import { bytesToBase64, MAX_IMAGE_BYTES } from './image-files';
import { ImageLink, imageLinks, parseNarrative, parseRelatedOutcome } from './related-format';

/** A request about an image while it runs as a job, and how it ended. */
export interface ImageJobState<T> {
  jobId: number | null;
  running: boolean;
  /** Why it failed (shown inline with Retry). */
  error: string | null;
  value: T | null;
}

/** A related-logs outcome shown without its image (opened from Jobs after the image was closed). */
export interface DetachedOutcome {
  jobId: number;
  imageName: string | null;
  outcome: RelatedOutcome;
}

/** "Add image support" of the active model: its download job. */
export interface ProjectorInstall {
  jobId: number | null;
  running: boolean;
  error: string | null;
}

interface ImageState {
  images: AiImage[];
  currentId: number | null;
  explanations: Record<number, ImageJobState<Narrative>>;
  relatedResults: Record<number, ImageJobState<RelatedOutcome>>;
  /** The image each job started here is about (the job's result does not say). */
  jobImages: Record<number, { imageId: number; imageName: string }>;
  detached: DetachedOutcome | null;
  opening: boolean;
  /** Why the last open or paste failed (Retry repeats it). */
  openError: string | null;
  /** A request was refused because the active model reads text only. */
  noVision: boolean;
  /** The active model as installed (for "Add image support (N MB)"). */
  activeModel: InstalledModel | null;
  projector: ProjectorInstall;
}

const initial: ImageState = {
  images: [],
  currentId: null,
  explanations: {},
  relatedResults: {},
  jobImages: {},
  detached: null,
  opening: false,
  openError: null,
  noVision: false,
  activeModel: null,
  projector: { jobId: null, running: false, error: null },
};

const idle = <T>(): ImageJobState<T> => ({ jobId: null, running: false, error: null, value: null });

/**
 * Images opened for the AI in this window (ADR 0025): the clean copies Rust made, the current one, its explanation
 * and related logs (AI jobs, followed while they run), and the no-vision state with "Add image support". The engine
 * decodes, searches and ranks; this keeps what the views show.
 */
export const AiImageStore = signalStore(
  { providedIn: 'root' },
  withState<ImageState>(initial),
  withComputed((s, status = inject(AiStatusStore)) => ({
    current: computed(() => s.images().find((i) => i.id === s.currentId()) ?? null),
    /** The active model cannot read images (its status says so, or a request was refused). */
    visionMissing: computed(() => s.noVision() || status.status()?.vision === false),
  })),
  withComputed((s) => ({
    currentExplain: computed(() => (s.currentId() !== null ? (s.explanations()[s.currentId()!] ?? null) : null)),
    currentRelated: computed(() => (s.currentId() !== null ? (s.relatedResults()[s.currentId()!] ?? null) : null)),
    /** The links of the shown outcome, as notes will hold them. */
    links: computed<ImageLink[]>(() => {
      const img = s.current();
      const related = img ? s.relatedResults()[img.id]?.value : null;
      if (img && related) return imageLinks(img.id, img.name, related);
      const d = s.detached();
      return d ? imageLinks(null, d.imageName, d.outcome) : [];
    }),
  })),
  withMethods((store, backend = inject(AiBackend), follow = inject(JobFollow), status = inject(AiStatusStore), owner = inject(DestroyRef)) => {
    let lastOpen: (() => Promise<AiImage | null>) | null = null;

    const setExplain = (id: number, patch: Partial<ImageJobState<Narrative>>) =>
      patchState(store, (s) => ({ explanations: { ...s.explanations, [id]: { ...(s.explanations[id] ?? idle<Narrative>()), ...patch } } }));
    const setRelated = (id: number, patch: Partial<ImageJobState<RelatedOutcome>>) =>
      patchState(store, (s) => ({ relatedResults: { ...s.relatedResults, [id]: { ...(s.relatedResults[id] ?? idle<RelatedOutcome>()), ...patch } } }));

    const opened = (img: AiImage, show: boolean) =>
      patchState(store, (s) => ({
        images: [...s.images.filter((i) => i.id !== img.id), img],
        currentId: show || s.currentId === null ? img.id : s.currentId,
        detached: show ? null : s.detached,
        opening: false,
        openError: null,
      }));

    const open = async (f: () => Promise<AiImage>, show: boolean): Promise<AiImage | null> => {
      const attempt = async () => {
        patchState(store, { opening: true, openError: null });
        try {
          const img = await f();
          opened(img, show);
          return img;
        } catch (e) {
          patchState(store, { opening: false, openError: toAiError(e).message });
          return null;
        }
      };
      lastOpen = attempt;
      return attempt();
    };

    /** Runs an image request as an AI job and follows it; the image keeps its state if the request fails. */
    const runJob = async <T>(imageId: number, start: () => Promise<number>, set: (id: number, p: Partial<ImageJobState<T>>) => void, parse: (r: unknown) => T | null) => {
      const img = store.images().find((i) => i.id === imageId);
      if (!img) return;
      if (store.visionMissing()) {
        patchState(store, { noVision: true });
        return;
      }
      set(imageId, { running: true, error: null, jobId: null });
      try {
        const result = await follow.run(start, owner, (jobId) => {
          set(imageId, { jobId });
          patchState(store, (s) => ({ jobImages: { ...s.jobImages, [jobId]: { imageId, imageName: img.name } } }));
        });
        if (!result) {
          // Cancelled: the last answer (if any) stays.
          set(imageId, { running: false });
          return;
        }
        const value = parse(result.type === 'narrative' ? result.narrative : result.type === 'relatedLogs' ? result.result : null);
        set(imageId, value ? { running: false, value, error: null } : { running: false, error: $localize`:@@ai.vision.unexpected:The answer could not be read.` });
      } catch (e) {
        const err = toAiError(e);
        // A job that failed reports its message only: a refused image request also changes the status (re-read it).
        if (err.code === 'noVision') patchState(store, { noVision: true });
        else void status.refresh();
        set(imageId, { running: false, error: err.message });
      }
    };

    const methods = {
      /** Opens an image file the user chose (Rust decodes it). `show` makes it the current image. */
      openPath(path: string, show = true): Promise<AiImage | null> {
        return open(() => backend.imageOpen(path), show);
      },

      /** Opens pasted or dropped bytes (base64, never decoded here). */
      pasteBytes(bytes: Uint8Array, name: string | null, show = true): Promise<AiImage | null> {
        return open(async () => {
          if (bytes.length > MAX_IMAGE_BYTES) throw new Error($localize`:@@ai.vision.tooLarge:The image is larger than 32 MB.`);
          return backend.imagePaste(bytesToBase64(bytes), name);
        }, show);
      },

      /** Opens a pasted or dropped file: its bytes, read without decoding. */
      async pasteFile(file: Blob & { name?: string }, show = true): Promise<AiImage | null> {
        if (file.size > MAX_IMAGE_BYTES) {
          lastOpen = () => methods.pasteFile(file, show);
          patchState(store, { openError: $localize`:@@ai.vision.tooLarge:The image is larger than 32 MB.` });
          return null;
        }
        const bytes = new Uint8Array(await file.arrayBuffer());
        return methods.pasteBytes(bytes, file.name ?? null, show);
      },

      /** Repeats the open or paste that failed. */
      retryOpen(): Promise<AiImage | null> {
        return lastOpen ? lastOpen() : Promise.resolve(null);
      },

      dismissOpenError(): void {
        patchState(store, { openError: null });
      },

      select(id: number): void {
        if (store.images().some((i) => i.id === id)) patchState(store, { currentId: id, detached: null });
      },

      /** Forgets an image here and in the engine. */
      async close(id: number): Promise<void> {
        await backend.imageClose(id).catch(() => false);
        patchState(store, (s) => {
          const images = s.images.filter((i) => i.id !== id);
          const explanations = { ...s.explanations };
          const relatedResults = { ...s.relatedResults };
          delete explanations[id];
          delete relatedResults[id];
          return { images, explanations, relatedResults, currentId: s.currentId === id ? (images.at(-1)?.id ?? null) : s.currentId };
        });
      },

      /** "Explain with AI": a narrative about the image, as an AI job. */
      explain(imageId: number, question: string | null = null): Promise<void> {
        return runJob<Narrative>(imageId, () => backend.start({ op: 'explainImage', imageId, question }), setExplain, parseNarrative);
      },

      /** "Find related logs": the engine searches the events the image shows, as an AI job. */
      findRelated(imageId: number): Promise<void> {
        return runJob<RelatedOutcome>(imageId, () => backend.start({ op: 'relatedLogs', imageId }), setRelated, parseRelatedOutcome);
      },

      /** A finished related-logs job opened from Jobs: with its image when it is still open, else on its own. */
      showOutcome(jobId: number, result: unknown): boolean {
        const outcome = parseRelatedOutcome(result);
        if (!outcome) return false;
        const of = store.jobImages()[jobId];
        if (of && store.images().some((i) => i.id === of.imageId)) {
          setRelated(of.imageId, { running: false, error: null, value: outcome, jobId });
          patchState(store, { currentId: of.imageId, detached: null });
        } else {
          patchState(store, { currentId: null, detached: { jobId, imageName: of?.imageName ?? null, outcome } });
        }
        return true;
      },

      /** Reads the active model (for the size of its image support). */
      async loadActiveModel(): Promise<InstalledModel | null> {
        try {
          const models = await backend.models();
          const active = models.find((m) => m.active) ?? null;
          patchState(store, { activeModel: active });
          return active;
        } catch {
          return null;
        }
      },

      /** Shows the no-vision message (a text-only model was asked about an image). */
      refuseNoVision(): void {
        patchState(store, { noVision: true });
        void methods.loadActiveModel();
      },

      dismissNoVision(): void {
        patchState(store, { noVision: false });
      },

      /**
       * "Add image support (N MB)": downloads the active model's catalog projector as a download job (only when the user
       * asks). Resolves when the job ended; the status then says the model reads images.
       */
      async installProjector(): Promise<void> {
        const m = store.activeModel() ?? (await methods.loadActiveModel());
        if (!m || store.projector().running) return;
        patchState(store, { projector: { jobId: null, running: true, error: null } });
        try {
          const jobId = await backend.installProjector(m.fileName);
          patchState(store, { projector: { jobId, running: true, error: null } });
          follow.started($localize`:@@ai.vision.projector.started:Downloading image support for ${m.name}:name: — running in Jobs`);
          const end = await follow.follow(jobId, owner);
          if (end?.state === 'failed') throw new Error(end.error ?? $localize`:@@ai.vision.projector.failed:The download failed.`);
          patchState(store, { projector: { jobId, running: false, error: null } });
          if (end?.state === 'done') {
            await status.refresh();
            await methods.loadActiveModel();
            patchState(store, { noVision: false });
          }
        } catch (e) {
          patchState(store, (s) => ({ projector: { jobId: s.projector.jobId, running: false, error: toAiError(e).message } }));
        }
      },
    };
    return methods;
  }),
);

export type AiImageStoreType = InstanceType<typeof AiImageStore>;
