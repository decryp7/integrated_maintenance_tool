# Investigation playbooks

A playbook is a saved, shareable runbook for an investigation ([ADR 0039](adr/0039-known-issues-incident-summary-playbooks.md)).
It is a JSON file; the library is a folder of them (`playbooks/` in the configuration folder, one `<id>.json` each), so
sharing one is copying a file or using *Export…* and *Import…* in the Playbooks pane. Four templates are built in.

```json
{
  "version": 1,
  "id": "service-crashed",
  "title": "A service crashed or restarted",
  "description": "Find when it stopped and what it logged just before.",
  "tags": ["crash"],
  "steps": [
    { "id": "fatal", "title": "Fatal events", "instruction": "Note the first one.",
      "action": { "type": "check", "ql": "level = fatal", "expect": { "kind": "noMatch" } } },
    { "id": "by-source", "title": "Which sources", "instruction": "One process or many?",
      "action": { "type": "query", "ql": "level >= error | stats count() as errors by @source | sort errors desc | head 10" } },
    { "id": "problems", "title": "Check Problems", "instruction": "",
      "action": { "type": "openView", "view": "problems" } },
    { "id": "before", "title": "Read what came before", "instruction": "Open the first fatal event in Moment.",
      "action": { "type": "manual" } }
  ]
}
```

| Field | Rules |
|-------|-------|
| `version` | `1`. A newer number is refused ("saved by a newer version"); a missing one means 1. |
| `id` | 1 to 60 of `a-z`, `0-9`, `-`; not starting or ending with `-`. Also the file name. |
| `title` | Required, at most 120 characters. `description`: at most 4 000. |
| `tags` | At most 10, each 1 to 40 characters. |
| `steps` | 1 to 100. Each has a unique `id` (same alphabet), a `title`, an optional `instruction` (4 000 characters) and an `action`. |

Actions (`action.type`):

| Type | Fields | What running it does |
|------|--------|----------------------|
| `manual` | none | Nothing; you do it and mark it done or skip it. |
| `query` | `ql` | Runs the query; shows the event count and sample events, or the table of a `stats` query (20 rows). |
| `filter` | `ql` | Checks the filter, counts its matches and sets it as the current view's filter. |
| `openView` | `view` | Opens a pane by its id (for example `problems`, `overview`, `query`). |
| `check` | `ql`, `expect` | Runs the query and passes or fails: `expect` is `{"kind":"noMatch"}`, `{"kind":"anyMatch"}`, `{"kind":"atLeast","n":N}` or `{"kind":"atMost","n":N}`. |

A query that does not fit the data (unknown field, bad syntax) ends the step with status *error* and the engine's
message; the rest of the run continues. Queries only read.

A **run** copies the playbook and records, for each step, the status (`pass`, `fail`, `info`, `error`, `skipped`),
the engine's summary line, the event count and samples, your note and the time. Runs are saved with the project
(UI state key `playbooks`, at most 50) and shared with it.
