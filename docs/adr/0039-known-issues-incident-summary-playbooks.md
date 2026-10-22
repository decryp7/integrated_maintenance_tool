# 39. Known issues, the incident summary and investigation playbooks

- Status: accepted (2026-10-22)
- Date: 2026-10-22
- Related: ADR 0007 (reports, PDF and redaction), ADR 0029 (knowledge files), ADR 0024 (the AI drafts search text),
  AGENTS.md (the engine is the source of truth; AI is optional and never trusted blindly)

## Context

Three features turn what the engine already knows into a workflow: recognising errors the user has already diagnosed,
producing a one-click write-up of an incident, and following a repeatable investigation. All three must work with no
AI model at all, and where a model helps it may only suggest or narrate: every fact stays computed by the engine.

## Decision

1. **Known issues are declarations in knowledge files.** A fenced block labelled `known-issue` holds `title`, `fix`,
   `id` (optional) and exactly one of `pattern` (a regex, compiled with `user_regex::compile`) or `query` (a
   query-language filter). `imt_core::known_issues` parses them (bounded: 100 per file, 200 evaluated per call, field
   lengths; a damaged block is skipped with a warning naming the file and what to fix) and matches them by running
   the filter through the store port. A match carries its count, five sample events and the exact query, so the UI
   opens the same events the engine counted. A pattern becomes `@message ~ "p" or @exception ~ "p"`.
2. **The AI may suggest, the engine verifies.** `Assistant::suggest_known_issues` shows the model the data's top
   exception groups and error patterns plus the notes and asks for `{title, fix, ql}`. The engine keeps a suggestion
   only if its `fix` is a quote from one of the notes (whitespace-insensitive), its query is valid and it selects at
   least one event. Suggestions are marked `origin: ai` everywhere and are not part of the incident report.
3. **The incident summary is an ordinary report.** `imt_core::incident::build_incident_report` returns a
   `Report` (ADR 0007): period, levels, key moments (anomaly windows, first occurrence of each exception type), top
   anomalies, exceptions and error patterns with sample events as evidence, sources and hosts with most errors,
   import problems, matched known issues with their fixes, and the project's notes. It is saved with the project's
   reports and exported by the existing HTML/PDF pipeline, so redaction, limits and SVG sanitising are not
   re-implemented. An analysis that fails leaves its section out; it never fails the report.
4. **The narrative is separate and marked.** `incident_narrative` feeds the engine's report digest (bounded, as
   data) to the model, which writes at most about 150 words citing events; citations are checked against the events
   the report and the tools returned. The UI inserts it as its own first section, labelled AI-written, with any
   unverified citation listed. Without a model the report is complete without it.
5. **Playbooks are versioned JSON files.** `imt_core::playbook::Playbook` (format `version: 1`, schema in
   `docs/playbooks.md`) has steps of five kinds: `manual`, `query`, `filter`, `openView` and `check` (a query and an
   expected outcome). The library is one `<id>.json` per playbook in the configuration folder's `playbooks/`
   directory (ids are file-name safe; a file whose id does not match its name is refused; writes are atomic), so
   sharing is copying a file; import and export go through the user's file dialogs. Playbooks from a newer format
   are refused with a message to update. Four templates are built in (valid and runnable on any data: a test runs
   every template step).
6. **Runs are recorded in the project.** `PlaybookRun` copies the playbook it ran (editing the library copy later
   does not rewrite history) and records, per step, the engine's status and summary, the count and sample events,
   and the user's note. The shell keeps no run state: the UI holds the `PlaybookRecord` in the project's UI state
   (key `playbooks`) and the engine validates it on every record and restore (newer or damaged records restore
   nothing and say why, damaged runs are skipped, at most 50 runs).
7. **Commands are stateless and thin** (`app/src-tauri/src/investigate`): they validate arguments, read the knowledge
   files the window sees (project files and notes, then global; only enabled ones), call the engine and, for the AI
   ones, check that a model exists first.

## Consequences

- Users get recognised problems, a shareable incident report and runbooks without any model; the model adds
  suggestions and a narrative that are verifiable.
- Knowledge text can now carry machine-read declarations; the format is deliberately small (one block, five keys).
- `query`-based issues depend on the field names of the data; one that does not fit is reported as a warning, not an
  error, so a knowledge file shared between projects keeps working.
- Run history lives in the project file; very long histories are bounded (50 runs, 100 steps per playbook).
