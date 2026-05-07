---
description: Verify algorithm implementations against the spec and cited papers
agent: build
---

Verify the algorithms in the current branch:

1. Dispatch `@algorithm-verifier` over `src/carve/`, `src/validate/`, `src/reassemble/`.
2. For each algorithm referenced in `.opencode/instructions/algorithms-spec.md`, confirm:
   - The implementation has a test that exercises the invariants from the spec.
   - Property tests obligated by the spec exist.
   - Default parameters match.
   - Any divergence has an ADR under `docs/decisions/`.
3. Output a table per algorithm: `name | spec section | impl location | tests | divergence ADR (if any) | verdict`.
4. If any algorithm shows silent divergence, status is BLOCKED.
