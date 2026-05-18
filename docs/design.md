# Design

## Architecture

`flashield-lite` is a single Rust CLI with small modules:

- `trace`: parses the supported CSV trace format.
- `lru`: implements a byte-capacity LRU cache used by DRAM and flash indexes.
- `flash`: simulates sequential flash segment writes and keeps a logical object
  index.
- `policy`: implements the cache policies.
- `metrics`: accumulates simulation counters and derived rates.
- `main`: parses CLI arguments, runs commands, and prints text or JSON reports.

The simulator loads a trace, feeds each event into one selected policy, finalizes
any buffered flash segment, and prints a report.

## Trace Model

The supported CSV schema is:

```csv
timestamp,op,key,size
```

Supported operations are:

- `get`: lookup by key. The `size` field should be `0`.
- `set`: create or replace an object with `size` bytes.
- `update`: update an object with `size` bytes.
- `delete`: remove an object. The `size` field should be `0`.

The parser is intentionally strict and does not support quoted CSV fields in v0.1.

## Workload Generator

`generate-trace` creates deterministic synthetic traces using a fixed seed. The
`--preset` option controls the operation mix:

- `mixed`: balanced default workload with reads, writes, updates, and deletes.
- `read-heavy`: emphasizes repeated `get` operations for stable hot objects.
- `update-heavy`: emphasizes repeated `update` operations to stress flash write
  admission decisions.

Unknown keys are always introduced with `set` before they can receive `get`,
`update`, or `delete` operations.

## Cache Policies

### DRAM-only LRU

`dram-lru` keeps objects in a byte-capacity DRAM LRU cache. `get` operations hit
only if the object is resident in DRAM. `set` and `update` insert or replace the
object in DRAM. Capacity pressure evicts least-recently-used objects.

### Naive Flash-backed Cache

`naive-flash` combines a DRAM LRU cache with simulated flash. Every `set` and
`update` is written to flash immediately. `get` checks DRAM first, then flash. A
flash hit is promoted back into DRAM.

This baseline intentionally writes aggressively so it can be compared against
Flashield-lite admission filtering.

### Flashield-lite

`flashield-lite` keeps new and recently modified objects in DRAM first. For each
DRAM object it tracks:

- `read_count`
- `update_count`
- `first_seen` timestamp, used to compute age
- `size`
- whether the object has already been admitted to flash

On reads and DRAM evictions, the policy checks whether the object looks stable and
read-worthy. If it passes the heuristic, it is appended to the flash segment
buffer and indexed in simulated flash.

Updates invalidate any old flash copy and mark the DRAM object as not currently
admitted. The object's `update_count` rises, so frequent changes suppress future
admission.

### Flashield-ML

`flashield-ml` keeps the same DRAM-first observation path, but replaces the final
fixed admission decision with a small online logistic model. The model scores an
object with normalized features derived from:

- read count,
- age,
- size,
- update count.

The score approximates the Flashield paper's flashiness idea: an object is a
better flash candidate when it is likely to be read again and unlikely to be
modified soon. Objects that survive in DRAM with enough reads become positive
training examples. Objects that are updated or deleted become negative examples.

This keeps v0.1 dependency-free while making the admission decision learned
rather than purely hand-coded.

## Admission Policy

The `flashield-lite` admission heuristic is:

```text
read_count >= min_reads
update_count <= max_updates
age >= min_age
```

This is deliberately simple. It captures the central intuition that objects with
some read evidence and few updates are better flash candidates than objects that
are being rewritten frequently.

The `flashield-ml` policy uses `min_reads`, `max_updates`, and `min_age` to label
online training examples. Admission itself is controlled by:

```text
flashiness_score >= ml_threshold
```

`--ml-learning-rate` controls how quickly model weights move after each labeled
example.

## Simulated Flash Model

The flash layer has two responsibilities:

1. Maintain a logical byte-capacity object index.
2. Charge physical bytes for sequential segment writes.

When a policy admits or writes an object, the simulator:

- increments `logical_bytes_admitted` by the object size,
- appends the object to a segment buffer,
- flushes full segments as the buffer fills,
- flushes a final partial segment as one full segment at the end of the run,
- inserts or updates the object in the logical flash index.

The logical flash index uses LRU eviction when its byte capacity is exceeded.

## Metrics

The report includes:

- total requests,
- lookup requests,
- cache hits and misses,
- hit rate over `get` operations,
- DRAM hits,
- flash hits,
- physical flash bytes written,
- logical bytes admitted to flash,
- write amplification,
- segment flushes,
- evictions.

Reports default to human-readable text. Passing `--output-format json` emits the
same counters as a machine-readable JSON object for scripts, dashboards, or
experiment notebooks.

## Experiment Scripts

The `scripts/run_experiments.ps1` and `scripts/run_experiments.sh` helpers run
the current policy matrix across the synthetic workload presets:

- `mixed`
- `read-heavy`
- `update-heavy`

For each preset, the scripts generate a trace under `traces/generated/` and run
`dram-lru`, `naive-flash`, `flashield-lite`, and `flashield-ml`. Reports are
emitted as JSON under `results/`.

`scripts/summarize_results.py` reads those JSON reports and writes
`results/summary.csv`, which is useful for comparing hit rate, flash bytes
written, and write amplification across policies. `scripts/summary_to_markdown.py`
converts that CSV into `results/summary.md` so the same comparison can be pasted
into a README, Wiki page, or PR description. The generated directories are
ignored because they are reproducible outputs, not source artifacts.

Write amplification is:

```text
flash_bytes_written / logical_bytes_admitted
```

If no bytes are admitted to flash, write amplification is reported as `0.0`.

## Limitations

- No real device I/O, latency, wear, erase blocks, or garbage collection.
- Segment writes are a simple accounting model.
- The final partial segment is charged as a full segment.
- The ML policy learns online from a single replay, not from a separate offline
  training corpus.
- The ML features are intentionally minimal and do not match the full paper.
- The parser supports only simple comma-separated fields without quoting.
- Flash capacity eviction is object-level LRU, not segment cleaning.
- The generator produces only synthetic keys and deterministic preset workloads.
