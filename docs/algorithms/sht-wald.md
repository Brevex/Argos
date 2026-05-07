# Sequential Hypothesis Testing (SHT / Wald SPRT)

## References

- Wald, A. (1945). *Sequential Tests of Statistical Hypotheses*. The Annals of Mathematical Statistics, 16(2), 117–186.
- Pal, A., Sencar, H. T., & Memon, N. (2008). *Detecting File Fragmentation Point Using Sequential Hypothesis Testing*. DFRWS 2008.

## Problem

While extending a candidate file block by block (PUP), decide whether the next block continues the current fragment or marks a fragmentation point — bounded by configurable false-positive and false-negative rates.

## Hypotheses

For block transition at step `n`:
- `H0`: the next block continues the current fragment.
- `H1`: the next block is a fragmentation point.

Let `x_n` be the observation at step `n` — the format-validator score for the candidate continuation. Let `f_0(x)` and `f_1(x)` be the densities of the validator score under `H0` and `H1`.

## Test statistic

Cumulative log-likelihood ratio:

```
S_n = Σ_{i=1..n} log( f_1(x_i) / f_0(x_i) )
```

## Decision boundaries

For chosen error rates α (Type I, accept H1 when H0 true) and β (Type II, accept H0 when H1 true):

```
A = log( (1 - β) / α )
B = log( β / (1 - α) )
```

Decision rule at step `n`:
- If `S_n ≥ A`: accept H1 (fragmentation point); finalize current fragment, leave PUP to seek a continuation.
- If `S_n ≤ B`: accept H0 (continuation); proceed.
- Otherwise: continue accumulating observations.

## Default parameters

- α = 0.01
- β = 0.01

These are configuration values, not magic numbers in code. They surface in `Config` and are validated to be in `(0, 0.5)`.

## Densities `f_0`, `f_1`

The densities depend on the validator:
- **JPEG (Huffman):** `f_0` derived from the empirical distribution of Huffman match-rate scores on contiguous JPEG fragments; `f_1` from scores on random or boundary-crossing data.
- **PNG (CRC):** `f_0` and `f_1` derived from the distribution of partial-chunk completion success.

These distributions are estimated from a calibration corpus and stored as constants. Recalibration changes are ADRs.

## Invariants

1. The decision is made strictly from accumulated observations. No look-ahead.
2. Thresholds `A` and `B` are derived once per session from the configured α, β. They are not tuned per file.
3. SPRT either decides within finite steps or PUP terminates the file due to other criteria (footer, max-length, exhausted candidates).

## Property tests (required)

- For synthetic inputs drawn from `f_0`, the false-fragmentation rate over many runs is `≤ α + ε` for a small ε.
- For synthetic inputs drawn from `f_1`, the missed-fragmentation rate is `≤ β + ε`.
- Boundary algebra: `A = log((1-β)/α)`, `B = log(β/(1-α))` for any (α, β) in the valid range.
- Decision is invariant under permutation of observations only when observations are exchangeable; the algorithm processes them in arrival order.
