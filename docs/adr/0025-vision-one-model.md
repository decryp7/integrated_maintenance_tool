# 25. Vision: one model reads text and images, the engine finds the related logs

- Status: accepted (2026-10-21)
- Related: ADR 0009 (prebuilt llama-server runtimes), ADR 0012 (jobs), ADR 0014 (store ports), ADR 0017 (one IPC:
  the worker protocol), ADR 0006 (clock skew applied on load), ADR 0021 (notes and image annotation, later),
  AGENTS.md (pure core / thin edges, parse don't validate, bounded resources)

## Context

Users attach screenshots to incidents: an error dialog, a frozen window, a console, a dashboard. They want the local
assistant to explain an image and, above all, to find the log events that belong to it. The PCs Debrief runs on are short
of memory (two machines ran out while a model and Debrief were loaded), so a second, vision-only model loaded next to the
text model is not an option. Upstream llama.cpp loads a multimodal projector (`mmproj` GGUF) into the same
`llama-server` as the language model (`--mmproj`): the one model then reads images too. Projectors are specific to a
model generation (a Qwen3.6 projector does not work with Qwen3.5), and llama.cpp reports crashes when speculative or
multi-token-prediction decoding is combined with image input.

Images are untrusted input like log files: crafted PNG/JPEG/GIF/WebP files exploit decoders, a "decompression bomb"
claims gigapixels in a few bytes, metadata carries locations and names, and the text on a screenshot can try to
instruct the model. Everything must stay on the PC.

## Decision

1. **One model, its own projector.** A catalog entry that reads images names its projector (`projector`: the
   `mmproj` file of the same repository, its SHA-256 and size). Installing the model downloads both, each verified
   like today; the projector is filed next to the model as `<model stem>.mmproj.gguf` (one naming rule for catalog
   and user-added models, so the pairing needs no state of its own and deleting the model deletes it). A projector is
   never a model: `gguf::parse` keeps refusing it ("choose the main model file, not the mmproj file"), and
   `gguf::parse_projector` accepts it only as a model's image encoder, after checking that its embedding width
   (`clip.vision.projection_dim`) equals the model's (`embedding_length`) — a projector of another generation is
   refused before llama-server sees it. "Add image support" installs the projector of an installed model (the catalog
   download, or a file the user picked, checked the same way). Never a separate vision model, never two models
   loaded, never an automatic download.
2. **The worker loads the projector into the same server.** Worker protocol 2 (ADR 0017): `LoadParams.mmproj_path`
   adds `--mmproj` and `LoadResult.vision` reports it. The projector goes to the GPU only when the whole model
   does (`--no-mmproj-offload` otherwise), so on an 8 GB GPU it stays in system RAM and the 9B model keeps its
   layers; recommendations judge the GPU fit on the model, memory checks count the projector;
   chat requests carry `images` (Debrief's sanitised encoding, base64; at most 4 images, 8 MB together, re-checked by the
   worker). Debrief renders prompts itself, so images go through llama-server's `/completion` multimodal prompt
   (`prompt_string` with llama.cpp's media marker per image, `multimodal_data` with the images) instead of the
   OpenAI-style `image_url` of `/v1/chat/completions`; the bytes are the same base64 a data URI would carry. The
   marker is stripped from every other text (logs, web pages, the system prompt), so no content can claim an image
   slot. The context check adds the image tokens (from the projector's patch and merge size) to `/tokenize`'s count.
   The worker never passes draft-model, speculative or MTP options (a test asserts it). Memory checks count the
   projector with the model.
3. **Capability, not guesswork.** `LlmProvider::vision()` (and the lazy loader's hint before the model is loaded)
   tells whether the active model reads images; AI status and the model list report "Text + images" or "Text only".
   A request with images to a text-only model fails with the typed `NoVision` error; the UI offers to install the
   projector of the active model (when the catalog has one) or to switch to a model marked "Text + images".
4. **Images are decoded in Rust only** (`debrief_ai::vision::image`, the `image` crate with only PNG, JPEG, BMP, GIF and
   WebP compiled in): the format by signature, the file size bounded (32 MB), the claimed dimensions checked before
   any pixel is decoded (64 MP, 16 384 px per side), the decoder's allocations limited (512 MB), GIF's first frame,
   decoder panics caught as "damaged". The pixels are flattened onto white, downscaled to what the model reads (its
   projector's patch × merge size and a budget of 1 024 image tokens, about a 1366×768 screenshot) and re-encoded
   from the pixels (PNG, or JPEG when much smaller): no EXIF, colour profile, extra frame or trailing bytes survive.
   The EXIF capture time is the one fact kept (a bounded TIFF reader). The webview never decodes the user's image for
   the AI path: it shows the clean copy (a `data:` URL, allowed by the CSP). Pasted and dropped images reach Rust as
   bytes, picked and Explorer images as paths the user chose (path grants). Clean images live in a small per-window
   cache (8 images) and are never written into chat history.
5. **"Find related logs": the model reads, the engine searches.**
   - The model fills a strict JSON schema (`ScreenFacts`, grammar-constrained): screen kind (error dialog, frozen
     window, console, dashboard, log viewer, browser, other), window title, application, shown times (with or
     without a date), error texts, error codes, exception names, machine, user and server names, ids, versions and
     a one-line summary. Rust parses it again (unknown fields refused, strings cleaned and cut, lists bounded and
     de-duplicated); an invalid answer gets one retry with the reason, then fails typed.
   - The engine operation `debrief_core::related::find_related` decides the time — shown date and time, else a shown
     time of day on the date of the other evidence (or of the data when it covers one day), else the EXIF capture
     time, else a time in the file name, else the file's modification time — in the profile's `assume_timezone`,
     and applies the clock offset of the screenshot's machine (a source named like a machine on the screen) the way
     ADR 0006 applies it to its logs. It searches through a small port (`RelatedSource`: sources, and events for a
     filter), implemented over the store's `Query` port (the database engine) and over the in-memory `LogStore`
     (today's app), with the events around the time and a query-language phrase search per strong clue across all
     time. Ranking is a pure function: time proximity (3 within the time's precision, falling to 0 at the window's
     edge, ±2 min by default) plus the clues matched (exception, code, id 3; error text 2.5, or 1.5 when most of its
     words match; title or application matching the source, the lane or a lane field 2; machine, user, server 1.5;
     version 1; errors +0.5 on an error screen). Every link carries typed reasons (`time +1.4 s`, `title matches lane
     OrderWorker`, `text 'HttpRequestException'`); notes say when there was no readable time (text and title only),
     when the time is outside the data, when the clocks likely disagree (the best strong match is outside the window:
     its distance and source, so the existing clock-offset correction can be offered) and when an offset was applied.
   - The model then explains the top matches in a few sentences, citing them; the text of the screenshot reaches it
     only as quoted data. The outcome is a typed result (`RelatedOutcome`) kept with the job, ready to become a
     "linked to image" note when notes arrive (ADR 0021).
6. **Jobs.** Explaining an image and finding its related logs are AI jobs (ADR 0012): queued with the other AI
   requests, cancellable, with stages as progress (reading the image is 10–60 s on a CPU). A chat question with an
   image runs as a chat turn with the image; a chat question that asks for the related logs ("find the logs related to
   this screenshot", `/related`) runs the related search as that turn.

## Consequences

- One install of the default model (Qwen3.5 9B) now downloads about 0.9 GB more (its F16 projector) and reads
  images; the 2B entry stays text only to keep its 8 GB promise. Qwen3.6 35B-A3B is offered as best quality, text +
  images, needing 32 GB of RAM.
- The worker protocol is at version 2; an old worker is refused at hello (the two always ship together).
- "Find related logs" works on both engines because it needs only `Query::page` and the source list; the store's
  `Search` port returns ids without the events, so phrase search goes through the query language, which every
  engine implements. When `Search` returns events (or the ports get "events by id"), the adapter can use it.
- The image viewer is minimal and reusable (an image and its actions); annotation comes with notes (ADR 0021).
- Images are never persisted with a chat; reopening a chat shows "[image]" in the question, not the image.
