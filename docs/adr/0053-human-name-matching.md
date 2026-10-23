# 53. Human-like name matching

- Status: accepted (2026-10-23)
- Date: 2026-10-23
- Related: the entity catalogue (`debrief_core::catalogue`, `entities`), `find`, the assistant's `find_values` and
  `find_events` tools, Debrief's own local AI model (optional)

## Context

The logs write one program as `CompanyA.Graphic.Builder.exe`, `graphicbuilder.exe`, `GraphicBuilder.exe` and
`graphic-builder`. The user asks about "graphic builder", "grafic bilder" or "image editor". A person knows these are one
thing, asks "which one?" when two things fit, remembers what they were told, and says why they think a name matches.
Matching on letters alone (exact, parts, prefix, trigrams) misses run-together spellings and synonyms and sometimes
suggests a wrong name with confidence.

## Decision

All of it lives in the engine (`debrief_core::names`); the UI and the assistant only show what it returns.

1. **Layer 1, deterministic and always on.** A Viterbi word segmenter (`graphicbuilder` is `graphic builder`) scored by a
   compact embedded word list authored for Debrief (about 1,000 common and software words, no third-party data, so no
   licence to carry) and by the workspace's own words (pieces of names that the logs separate, which are cheaper, so
   splits follow the logs). Names are normalised (file name of a path, extension, repeated vendor prefix, version and
   architecture suffixes), and spellings group into one entity with a canonical spelling and counts per spelling.
   Bounded (64 characters, 8 words), pure, never panics; unknown tokens (`svchost`) stay whole.
2. **Layer 2, optional AI aliases.** A low-priority background job (`alias_job`) asks Debrief's own local model (the
   llama.cpp worker the assistant uses, through `debrief_ai::names_model::LocalAliasModel`) once per entity above a
   frequency threshold and per model version for a human name and up to six aliases, as strict JSON. The shell runs it
   as a quiet Maintenance job (its own queue, no message or notification), after the catalogue is built, and waits for
   the assistant's queue to be empty before every request. It never loads a model by itself: it runs only when the
   model is in memory or is loaded automatically. Every answer is validated (length, charset, word count, no echo of
   the prompt, no duplicates), cached in the name book, never requested at search time, labelled AI-generated,
   removable by the user, and switchable off in Settings. Nothing leaves the machine; without a model the layer is off.
   (Pitwall, the separate local model server, is not used here; it joins later through its app contract.)
3. **Context.** Candidates are ranked by the selected time window, open sources, the event in the Inspector, recent
   searches and co-occurrence (names in the same events), which also joins look-alike spellings.
4. **Ask when unsure.** Two different entities that match in the same way with scores closer than `TIE_GAP` produce a
   short question instead of a guess.
5. **Learn from the user.** Accepting or rejecting a match, an alias, a rename, a merge and a split are stored as user
   facts, per workspace or global, with undo. They outrank AI aliases and the AI job never overwrites them.
6. **Explain.** Every match carries how it matched (`userAlias`, `exact`, `segmented`, `parts`, `prefix`, `alias`,
   `fuzzy`), a one-phrase reason, all its spellings and a few evidence event ids.

## Not decided here: embeddings

Semantic (vector) search over the entity names was considered as a third layer and **deliberately left out** by the user
(2026-10-23): no embedding storage, no vector search, and no exception to "no vector search in Debrief". The
evaluation below shows that layers 1 and 2 with context already reach the target. Revisit only if real logs show a gap
the aliases cannot close, and then as its own ADR.

## Evaluation

`crates/debrief-core/tests/names_eval.rs` runs a labelled set of invented names (spelling variants, run-together words,
typos, abbreviations, synonyms, ambiguities, hard phrasings, negatives) against the layers, with fixed alias fixtures in
place of a model, and asserts the figures (top-1, top-3, false-suggestion rate). `tests/names_perf.rs` (ignored by
default) times segmentation, grouping, resolution and the alias job at 50,000 names.

## Consequences

- The catalogue gained `CatalogueRequest::all` so the name book can be built from every name; the book is a versioned
  JSON file that tolerates a missing or corrupt file.
- The word list is small; a name made of words it lacks stays whole until the workspace shows the words separated.
- The assistant's tools return `why`, `spellings`, `evidence` and `ask_the_user`, and are told to relay them.
