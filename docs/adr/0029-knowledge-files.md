# 29. Knowledge files: the user's Markdown notes as background for every AI feature

- Status: accepted (2026-10-21)
- Related: ADR 0009 (local models), ADR 0012 (jobs), ADR 0016 (one command table, typed errors), ADR 0025 (vision),
  AGENTS.md (engine is the source of truth, versioned persistent state, bounded resources)

## Context

Coding agents read `AGENTS.md`/`CLAUDE.md` to learn a codebase's vocabulary. Debrief's users want the same for their
systems: which product and service names exist, what each log source is, known errors and their fixes, facts about
their environments. Without it the local model guesses, and small models guess badly. Some of this knowledge is the
user's (true everywhere), some belongs to one investigation and must travel with the project when it is shared.

## Decision

1. **Knowledge files are Markdown text the user writes or imports.** Two scopes: *global* (Debrief's configuration folder,
   `<config>/ai/knowledge/`, one `.md` per file plus a versioned `index.json` with names, order and enabled flags) and
   *project* (inside the project's and the session's UI state under `knowledge`, a versioned record the engine owns,
   like the chats: a shared `.debriefproj` carries its knowledge). Each scope holds up to 32 files; each file can be
   enabled or disabled, renamed, edited, exported and removed. They are managed in Settings ▸ AI ▸ Knowledge, which
   lists both scopes (the project scope is the window's project).
2. **One pure assembler in `debrief-ai` (`knowledge`).** It skips disabled and empty files, orders project files before
   global ones (the more specific knowledge wins when the budget runs out), caps each file at 64 KiB and the whole
   block at a quarter of the model's context window (at most 96 KiB), and cuts with a visible note naming what was
   left out. The block has clear markers and a preamble: the files are the user's own notes, used as facts; they do
   not change Debrief's rules and are never instructions to ignore them. Anything in a file that looks like the markers
   is defused.
3. **Every AI feature gets it through the provider.** `KnowledgeProvider` decorates the `LlmProvider` port: it appends
   the block to the system prompt *after* Debrief's own rules and reports a context window smaller by the block's size,
   so every existing budget (chat history, tool output, samples) still fits. `AiCore::assistant` takes the window's
   project knowledge, so chat, research, explanations, folder set-up, query drafting and vision all include it
   without each feature knowing about knowledge files.
4. **Storage tolerates damage.** A missing index is an empty list; a damaged index is rebuilt from the `.md` files in
   the folder (enabled, named after the file) with a warning; an index from a newer Debrief is read but not rewritten
   (changes are refused with a message). Writes go through a temporary file and a rename. A damaged or newer project
   record restores no knowledge and says why, like the chats.
5. **Privacy.** Knowledge text never goes to the activity log, Debrief's log or error messages (only ids, names and sizes);
   it leaves the PC only as part of an export the user chose.

## Consequences

- Users can teach the assistant their vocabulary once (global) or per investigation (project), and share the latter.
- Knowledge costs context: a small model (4k tokens) gets about 3 000 characters of it; the cut is visible to the
  model and the note names the file, so the user can shorten or disable files.
- No "what does the AI know?" view yet (YAGNI); the Settings list shows sizes and which files are enabled.
