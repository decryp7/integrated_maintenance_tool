# 33. Video sources with a downloadable FFmpeg runtime

- Status: accepted (2026-10-22)
- Date: 2026-10-22
- Related: ADR 0009 (prebuilt runtimes downloaded on demand), ADR 0012 (jobs), ADR 0014 (store ports, projects),
  ADR 0017 (child processes and their sandbox), ADR 0006 (clock skew), ADR 0025 (vision on the one model),
  ADR 0021 (notes), AGENTS.md (pure core / thin edges, bounded resources, degrade don't fail)

## Context

Users record their screen while a problem happens: a dialog that appears, a progress bar that stops, a page that
never loads. They want those moments next to the logs: what the screen showed at 10:42:07 and which log events belong
to it. A recording is minutes of mostly identical frames (a blinking cursor, a clock ticking in the task bar) with a
handful of moments that matter.

Decoding video means FFmpeg. Linking it (libav*) would put a large C code base that parses untrusted input into Debrief's
process, need a C toolchain in every build, and tie Debrief's licence to the FFmpeg build's. Bundling the binaries in the
installer adds 50–180 MB per platform that most users never need. llama.cpp's runtime (ADR 0009) already solved the
same shape of problem: pinned official builds, downloaded on demand, verified, self-tested, run as a separate process.

## Decision

### 1. An FFmpeg runtime, managed like the inference runtime

- **Pinned LGPL builds.** `crates/debrief-video/src/runtime/ffmpeg-runtimes.json` names one release of BtbN's
  FFmpeg-Builds (the builds FFmpeg's download page links for Windows and Linux) and for every platform its
  `lgpl-shared` asset: URL, size and SHA-256 (from the GitHub release API's asset digest; `scripts/update-ffmpeg-runtimes.py`
  regenerates the file and `--verify` recomputes the hashes by downloading). Pinned: a month-end autobuild (those are
  kept for about two years; daily builds are deleted within weeks), release branch n9.0, for `windows-x64`,
  `windows-arm64`, `linux-x64` and `linux-arm64`. Only LGPL builds are pinned: no GPL components (x264, x265) are
  ever downloaded by Debrief, and Debrief never links or redistributes FFmpeg.
- **macOS has no pinned build yet.** No publisher offers LGPL macOS builds with stable URLs and published checksums
  (evermeet.cx and the other macOS builds are GPL, or rolling). On macOS the runtime manager says so and offers the
  user-provided path (e.g. Homebrew's `ffmpeg`), validated by the same self-test. The catalog has `macos-arm64` and
  `macos-x64` slots, so a pin is a data change once a trustworthy source exists.
- **Install.** Download with the model downloader (free-space check, resume, retries, size and SHA-256 before use) from
  `github.com` and its release CDN only, then unpack defensively (`runtime::unpack`: zip and tar.xz; only `ffmpeg`,
  `ffprobe`, their shared libraries and licence files; flat names, no `..`, no links outside the folder, caps on
  entries and sizes) into `<data>/video/ffmpeg/.staging-<release>`.
- **Self-test before use.** The staged build analyses a tiny clip Debrief generates itself (`runtime::testclip`: a
  64×48, 5 fps, six-frame Y4M with a block that moves each frame; no real footage, nothing bundled): ffprobe must report
  six frames and 1.2 s, and the frame pass must keep exactly the six frames at 0, 0.2, … 1.0 s with their PNGs. Only a
  build that passes is renamed to `<data>/video/ffmpeg/<release>/`.
- **Update with rollback.** An update installs the new release next to the current one; the current one stays in use
  until the new one passed its self-test. The release before is kept as the one rollback target ("Roll back"), older
  ones are removed. A failed self-test leaves everything as it was and the job fails with `selfTestFailed`, naming the
  step. A missing or damaged binary of the active release is found by the status check and reported with "Install
  again".
- **A user-provided ffmpeg.** Settings ▸ Video can point at a folder or an `ffmpeg` executable (with `ffprobe` next to
  it); Debrief runs the same self-test on it before it is used and shows the version it reports. A path that fails stays
  saved but unused, with the reason.

### 2. FFmpeg runs as a sandboxed, capped, cancellable process

`debrief_video::run` starts `ffmpeg`/`ffprobe` through the one child-process launcher (`debrief_ipc::process`,
`Containment::Launcher`): an empty environment (only `FFREPORT`, which makes FFmpeg write its log to a file), no
network (AppContainer without network capabilities on Windows, seccomp on Linux, the sandbox profile on macOS),
reading only the runtime folder and the chosen video, writing only a fresh temporary folder, no child processes,
below-normal priority. The host's memory watcher (`SandboxSpec::memory_limit`, 1 GiB by default) stops a process
that uses more; a wall-clock limit per step (probe 30 s, analysis and extraction scaled with the video's length, at
most 30 min) and Cancel kill it. Arguments are always a list (no shell), `-nostdin`, the input is an absolute,
canonical path the user chose. On Windows the AppContainer is granted read access to the video file itself; when the
file's permissions cannot be extended (a share, a file the user does not own) the process runs in the job object with
the same limits but without the AppContainer, and the job says so. Output is parsed from files in the temporary
folder (`ffprobe -o`, the `showinfo` lines of the report, the `metadata` filter's file), never from a pipe that
could block. Each video is analysed as a job (kind *analysis*) with progress and Cancel in the Jobs pane.

### 3. Frame selection

Two FFmpeg passes and a pure selection in between (`debrief_video::select`):

1. **Candidates.** The video, scaled to 256 px wide, goes through `mpdecimate` (drops frames that differ from the
   last kept one only in small blocks: a blinking cursor, a ticking clock, mouse movement) and `select` with the
   scene score of every remaining frame, written by the `metadata` filter with its exact time.
2. **Selection** (pure, tested): the first and the last candidate always; in *changes* mode (the default) every
   candidate whose scene score reaches the threshold (0.10); while that is more than the cap (30, at most 120), the
   threshold rises by a quarter, up to 0.60; still too many, the kept ones are spread evenly over time and a note says
   that and how many changes were not kept. In *interval* mode one frame per interval (default 10 s), the candidate
   nearest each point, without duplicates.
3. **Extraction.** One pass decodes exactly the selected times (`select` on `t`), scaled for the vision model
   (at most 1 366 px a side), `showinfo` reporting each frame's exact time. Each PNG is decoded and re-encoded in
   Rust (`debrief_ai::vision::image::sanitize`, ADR 0025): only clean pixels are stored.

The user sees the frame strip before anything goes to the model: they can drop frames, add the frame at the current
player position (one more extraction), or switch modes and analyse again.

### 4. Storage: the project, no new database

- Everything of a video lives in the project (store) folder under `videos/<key>/` (key: the first 16 hex digits of
  the video's SHA-256): `video.json` (a versioned `VideoRecord`: name, size, SHA-256, duration, size in pixels, the
  container's creation time, the alignment, the selection settings and notes, and every kept frame with its video
  time, image file, description and note), `frames/<n>.png|jpg`, and the frames source `<name>.frames`. A project
  export (`.debriefz`) carries the folder like any other project file; a damaged or newer `video.json` is skipped with a
  warning, never a failure.
- **The video itself is referenced** by name, size and SHA-256 (and its path on this machine) by default. "Include the
  video in the project" (per video, showing its size) copies it to `videos/<key>/media/`; the player then plays that
  copy. A referenced video that moved can be located again; it is accepted only with the same SHA-256.
- **Each video is a source** in the existing store: `<name>.frames` is read by a built-in importer
  (`debrief_core::importers::VideoFramesImporter`, id `video-frames`; JSON lines with a typed header) into one event per
  kept frame: time = the aligned moment, message = the AI description (or "Frame 3 at 00:01:23.400" before there is
  one), fields `Video`, `Frame`, `VideoTime`, `Note`, `AlignedBy`, `Confidence`; `Video` is its identity field, so the
  video is its own lane. The timeline, search, filters, Moment, notes, exports and the SQL console work unchanged.
  When frames, descriptions, notes or the alignment change, the file is rewritten and the source is replaced.
- **Bounds.** At most 120 frames per video, 600 per project, 6 h of video, 2 000 characters per description or note.
  Removing a video asks first and removes its source, frames and folder (an included copy too).

### 5. Alignment to the logs

`debrief_video::align` (pure) decides the moment of video time 0 from, in order of trust: a start the user set (or an
offset they applied: *user*, high confidence), a clock on screen read by the vision model (`read_screen` of ADR 0025 on
a chosen frame, resolved by `debrief_core::related::time::resolve` in the profile's zone: *on-screen clock*, high with a
date, medium for a time of day dated by the container), the container's `creation_time` (*container metadata*,
medium: some recorders write the start, others the end), the file's modification time minus the duration (*file
time*, low). Every frame event carries the method and confidence; the Video pane shows them with the evidence ("the
clock in frame 4 read 10:42:07"). The frames source takes clock offsets like any other source (ADR 0006).

The order of automatic evidence is therefore: on-screen clock, then a date and time in the file name (screen
recorders name files by their start), then the container, then the file time. A user-provided ffmpeg on macOS (e.g.
Homebrew's) lives outside the runtime folder: its installation prefix (e.g. `/opt/homebrew`) is added to what the
sandbox may read, so its libraries load; the prefix must be readable by the sandbox profile.

### 6. AI

The one loaded model describes frames (ADR 0025): one request per kept frame with its video time, as an AI job with
a stage per frame, cancellable; a description is at most two sentences of what is visible. The screen's text is
untrusted data: it only becomes event text, never instructions. A model without vision is never sent images: the job
fails fast with `NoVision` and the pane offers to install the model's projector or switch models. "Explain with AI"
on a frame and "Find related logs" put the clean frame into the window's image cache with its aligned time as
evidence and run the existing image use cases; "Explain" on a range sends up to four frames of it in one request.

### 7. UI

A *Video* view: the player (the local file through a CSP-safe `media:` scheme the shell serves with range requests,
only for videos open in that window, never a remote URL), the frame strip with selection, notes on frames, the
alignment with its provenance, Explain and Find related logs. Videos open through File ▸ Open and drag and drop like
other files (the shell sniffs them by signature). Settings ▸ Video manages the runtime.

## Consequences

- Debrief's installer and build stay free of FFmpeg; a user who analyses videos downloads about 55–80 MB once.
- The two-pass analysis reads the whole video twice at reduced size; on a laptop that is about a tenth of real time
  for a 1080p screen recording. The cap and the adaptive threshold keep the AI work bounded (30 descriptions).
- Pinning a new FFmpeg release is a reviewed data change that must pass the self-test on CI's platforms.
- On macOS the user provides FFmpeg until an LGPL source with checksums exists.
- The Windows AppContainer grant adds one access entry for Debrief's container to the chosen video file.
