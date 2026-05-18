# Experiment Guide

This guide explains how to compare the cache policies and read the results. The
default experiment matrix is intentionally small so it can run on a laptop and be
easy to inspect in a portfolio review.

## Policies

The experiment scripts run four policies:

- `dram-lru`: DRAM-only baseline. This shows the hit rate available without
  using flash.
- `naive-flash`: flash-backed baseline. This writes every `set` and `update`,
  so it usually has the highest flash write volume.
- `flashield-lite`: DRAM admission filter with fixed thresholds.
- `flashield-ml`: DRAM admission filter with a small online flashiness model.

## Workloads

The scripts generate deterministic synthetic traces for three workload shapes:

- `mixed`: balanced reads, writes, updates, and deletes.
- `read-heavy`: stable objects are read repeatedly. Admission policies should
  preserve hit rate while admitting useful objects to flash.
- `update-heavy`: objects are rewritten frequently. Admission policies should
  reduce flash writes compared with `naive-flash`.

## Running the Matrix

On Windows PowerShell:

```powershell
./scripts/run_experiments.ps1
```

On Unix-like shells:

```bash
./scripts/run_experiments.sh
```

The scripts write reproducible outputs under `results/`:

- one JSON report per workload and policy,
- `summary.csv` for spreadsheet or plotting workflows,
- `summary.md` for README, Wiki, and PR descriptions.

The generated `results/` and `traces/generated/` directories are ignored by Git.

## Markdown Summary

To regenerate only the Markdown table from an existing CSV summary:

```bash
python scripts/summary_to_markdown.py --input results/summary.csv --output results/summary.md
```

The table focuses on the counters that explain the tradeoff:

- hit rate,
- flash bytes written,
- logical bytes admitted,
- write amplification,
- flash hits,
- evictions.

## Reading Results

Start with `naive-flash`. It is the write-heavy reference point because every
write-like operation reaches flash. Then compare `flashield-lite` and
`flashield-ml` against it:

- If flash bytes written fall while hit rate stays close, admission filtering is
  doing useful work.
- If write amplification rises sharply, the segment size may be too large for
  the amount of data admitted in that trace.
- On `update-heavy`, a good admission policy should avoid most writes that are
  quickly invalidated.
- On `read-heavy`, a good admission policy should keep enough stable objects in
  flash to avoid a large hit-rate drop.

`dram-lru` is useful as a lower-bound storage baseline. It writes no flash data,
but it also cannot serve flash hits after DRAM eviction.

## ML-Specific Interpretation

`flashield-ml` is not trained offline before the run. It learns online while the
trace is replayed. That means early decisions are driven mostly by the initial
weights, while later decisions reflect the objects it has already observed.

The useful comparison is not whether ML always beats the heuristic. The useful
question is whether a learned flashiness score can approach the fixed heuristic
while creating room for richer features in future versions.

## Current Limitations

- The traces are synthetic and deterministic.
- The ML model is intentionally tiny and dependency-free.
- There is no separate training, validation, or test split.
- The simulator models flash write accounting, not real device latency or wear.
