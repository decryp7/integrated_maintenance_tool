# 34. Error snapshots: native webview capture instead of rasterising the DOM

- Status: accepted (2026-10-22)
- Related: ADR 0028 (native feel), ADR 0024 (docked browser), AGENTS.md (long-running work off the UI thread, Rust
  is the source of truth, bounded resources), `app/src-tauri/src/window_capture/`, `app/src-tauri/src/snapshots.rs`,
  `app/src/app/diagnostics/error-snapshot.service.ts`

## Context

An error snapshot is a picture of the window plus the error's context, saved locally when something goes wrong or
when the user presses *Capture diagnostic screenshot*. Until now the picture was drawn in the page with
html-to-image: clone the whole DOM, copy every computed style property onto every clone, serialise it into an SVG
`foreignObject`, load that as an image and draw it into a canvas. All of it runs on the page's main thread. On a
loaded workspace (thousands of grid cells, the timeline and charts as canvases, Angular Material's theme tokens)
this froze the window for 3–8 s, at the very moment something had just gone wrong. An earlier fix (dropping the
`--mat-*` tokens from the copied properties) only shortened the freeze. The approach also misses what is not in
the DOM tree it clones: the docked browser's page is its own webview (ADR 0024) and is never in the picture.

Two ways out were compared.

### A. A newer JS library in a worker (`modern-screenshot`)

`modern-screenshot` (MIT) is a maintained fork of html-to-image with the same technique. Its `workerUrl` option moves
*resource fetching and encoding* into web workers, but a worker has no DOM: cloning nodes and reading
`getComputedStyle` for each of them, which is where the time goes, still happen on the main thread. It would be
somewhat faster (it caches styles per tag) and keep the same blind spots: fonts must be skipped (the CSP forbids the
`fetch` the font embedder needs), cross-origin and separate webviews are blank, and the result is a re-rendering of
the DOM, not what the user saw (differences in canvas state, scroll positions, `backdrop-filter`, etc.). It adds an
npm dependency, a worker script that the CSP must allow, and keeps the work proportional to the size of the page.

### B. Ask the platform webview for a picture

Every webview IMT runs on can snapshot itself asynchronously, from what it has already composited:

| Platform | API | Result | Threading |
| --- | --- | --- | --- |
| Windows | WebView2 `ICoreWebView2::CapturePreview(format, IStream, handler)` | PNG or JPEG written into a stream | called on the UI thread, returns at once; WebView2 encodes in its browser process and calls the handler on the UI thread |
| macOS | `WKWebView takeSnapshotWithConfiguration:completionHandler:` (10.13+) | `NSImage` | called on the main thread, completion block on the main thread |
| Linux | WebKitGTK `webkit_web_view_get_snapshot(region, options, cancellable, callback)` | `cairo_surface_t` (ARGB32) | GAsync: callback on the GTK main loop |

All three are reached through Tauri's `Webview::with_webview`, which hands over the platform handle on the UI thread.
The bindings are already in IMT's dependency tree: `webview2-com` (MIT) is a direct Windows dependency, wry links
`objc2`/`objc2-web-kit`/`objc2-app-kit`/`block2` (MIT) on macOS and pins `webkit2gtk` 2.0.2 and `cairo-rs` 0.18
(MIT) on Linux, and tauri pulls in `image` 0.25 (MIT OR Apache-2.0) with the PNG, JPEG and BMP codecs. Using them
directly adds no crate to `Cargo.lock` and no licence beyond `deny.toml`'s allow-list.

## Decision

Use the platform capture (B). The page no longer rasterises anything; html-to-image is removed.

1. **One command, `capture_window_image`** (`app/src-tauri/src/window_capture/`, a query: no side effects). It captures
   the *calling* window's webview and replies `WindowCapture`: `captured { image (base64), mime, bytes }` or
   `unavailable { reason }`. The UI passes the image to `save_error_snapshot` exactly as before, so the store, its
   validation (`validate_image`: mime, size, magic bytes) and its retention are unchanged.
2. **Never block a thread on the webview.** `with_webview` only *starts* the platform call on the UI thread and returns.
   The completion callback does nothing but copy bytes into a one-slot channel: WebView2's PNG stream, macOS's image
   as BMP (an uncompressed copy, so no compression runs on the main thread), WebKitGTK's surface pixels. The command
   awaits the channel asynchronously with a 10 s timeout (`CAPTURE_TIMEOUT`); a webview that goes away without
   answering drops the channel and the command answers at once.
3. **Bound and encode off the UI thread** (`window_capture::bound`, pure and unit-tested, on `spawn_blocking`):
   decode with limits (64 MiB of input, 16 384 px per side, 512 MiB of decoder memory, PNG/BMP/JPEG only), convert
   premultiplied BGRA where needed, scale to at most 1600 px wide, keep a PNG up to 2 MiB, otherwise a JPEG at quality
   80, and halve the width until it fits the store's 3 MiB limit.
4. **Degrade, don't fail.** Any reason a picture can't be taken (unsupported platform, the webview refused, timeout,
   undecodable data, too large) is `unavailable` with a message, not an error: the snapshot is still saved, with a
   transparent 1 × 1 placeholder and "No screenshot: …" appended to its message, because the error's context matters
   more than the picture.
5. **The UI stays a client.** `ErrorSnapshotService` asks through `SnapshotBackend.captureWindow()` (never `invoke`);
   the mock backend returns a small placeholder PNG at once. While the capture is pending, `<html>` carries
   `data-snapshot-capturing` and elements marked `data-snapshot-ignore` (toasts) are hidden by a global rule, which
   is what html-to-image's node filter did.

## Consequences

- No freeze: capturing costs the page one awaited promise. A test asserts that the UI keeps handling input and
  rendering while a capture is pending, and that no canvas, serialiser or DOM clone is used.
- The picture is what the webview composited, including canvases exactly as drawn. It covers the window's main
  webview only; the docked browser's page (a separate webview) is still not in it, as before. Native window chrome
  (title bar) is not included either, which matches the old picture.
- Platform code is behind `cfg`: Windows is built and tested here; the macOS part type-checks against the
  `x86_64-apple-darwin` target in isolation and the Linux part relies on the Linux CI runner (the GTK system
  libraries are needed to build it). Other targets answer `unavailable`.
- The images are now encoded by the `image` crate rather than the browser's canvas encoder; sizes are comparable.
- `mockSnapshots=off` in e2e tests stays, but only to keep snapshot toasts out of tests about other error states:
  the mock no longer renders the page.
