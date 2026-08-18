# Audit prompts

Prompts for audits run against this repository by a session outside it. Nothing runs them: a person
pastes one into a fresh session and keeps the report beside it.

They exist because the questions they ask cannot be answered from inside the work. An audit of
whether a recovery pipeline matches the published state of the art is worth nothing if it is
conducted by the same reasoning that built the pipeline, and worth less still if its conclusions
carry no provenance. Each prompt therefore fixes an **evidence contract** before it asks anything —
what counts as measured, as read from code, as published, as inferred, and as unknown — and states
what would refute its own conclusion.

Like `docs/defects/`, these are written once, for the question they were written for, and are not
revised afterwards (`M-NO-META-DESIGN-DOCUMENTATION`). A report produced by one is kept next to it.

| Prompt | Asks | Report |
| --- | --- | --- |
| [PROMPT-RECUPERABILIDADE.md](PROMPT-RECUPERABILIDADE.md) | Whether a batch of photographs deleted inside Windows in ~2022, on a magnetic disk reformatted since, is still recoverable — and whether Argos leaves any residue of them unread | [RELATORIO-RECUPERABILIDADE.md](RELATORIO-RECUPERABILIDADE.md) — first pass, **not independent**: run by the session that wrote the prompt |
