# flashield-lite

[![Rust CI](https://github.com/dldl8819/flashield-lite/actions/workflows/ci.yml/badge.svg)](https://github.com/dldl8819/flashield-lite/actions/workflows/ci.yml)

`flashield-lite` is a small Rust research and portfolio project inspired by the paper
*Flashield: a Hybrid Key-value Cache that Controls Flash Write Amplification*.

This is not the official Flashield implementation. It is a simplified trace-driven
simulator that demonstrates the core idea: DRAM can act as an admission filter so
objects that are likely to be updated or discarded soon do not immediately consume
flash write bandwidth.

## Paper Reference

- *Flashield: a Hybrid Key-value Cache that Controls Flash Write Amplification*

## Implemented

- CSV trace parser for:

  ```csv
  timestamp,op,key,size
  1,set,synthetic:1,128
  2,get,synthetic:1,0
  3,update,synthetic:1,256
  4,delete,synthetic:1,0
  ```

- Cache policies:
  - `dram-lru`: byte-capacity DRAM-only LRU cache.
  - `naive-flash`: writes every `set` and `update` to simulated flash.
  - `flashield-lite`: stores objects in DRAM first, tracks simple per-object
    features, and admits stable read-worthy objects to simulated flash.
- Simulated flash storage:
  - logical object index with byte capacity,
  - physical bytes written,
  - sequential segment buffering,
  - segment flush counts.
- Text and JSON simulation reports.
- Deterministic synthetic trace generator.
- Unit tests for parsing, LRU eviction, flash accounting, admission, and workload
  behavior.

## Simplifications

- No real SSD I/O is performed.
- The admission model is a heuristic, not a learned model.
- Flash garbage collection, erase blocks, wear leveling, and device latency are not
  modeled.
- CSV parsing intentionally supports only the documented simple trace format.
- Hit rate is measured over `get` operations. `set`, `update`, and `delete` count
  toward total requests but not lookup hits or misses.

## Admission Policy

`flashield-lite` admits an object to flash when all of the following are true:

- `read_count >= min_reads`
- `update_count <= max_updates`
- `age >= min_age`

Defaults:

- `min_reads = 2`
- `max_updates = 1`
- `min_age = 2`

## Running

Generate a synthetic trace:

```bash
cargo run -- generate-trace --output traces/sample.csv --requests 10000
```

Run the DRAM-only baseline:

```bash
cargo run -- simulate --policy dram-lru --trace traces/sample.csv
```

Run the naive flash-backed baseline:

```bash
cargo run -- simulate --policy naive-flash --trace traces/sample.csv
```

Run Flashield-lite:

```bash
cargo run -- simulate --policy flashield-lite --trace traces/sample.csv --dram-capacity 1048576 --flash-capacity 10485760 --segment-size 1048576
```

Print a machine-readable JSON report:

```bash
cargo run -- simulate --policy flashield-lite --trace traces/sample.csv --output-format json
```

Optional Flashield-lite knobs:

```bash
cargo run -- simulate --policy flashield-lite --trace traces/sample.csv --min-reads 3 --max-updates 0 --min-age 5
```

Run tests:

```bash
cargo test
```

## Example Output

Text report:

```text
Policy: flashield-lite
Total requests: 10000
Lookup requests: 5012
Cache hits: 2419
Cache misses: 2593
Hit rate: 48.26%
DRAM hits: 1834
Flash hits: 585
Flash bytes written: 2097152
Logical bytes admitted: 1636288
Write amplification: 1.28
Segment flushes: 2
Evictions: 0
```

JSON report:

```json
{
  "policy": "flashield-lite",
  "total_requests": 10000,
  "lookup_requests": 5012,
  "cache_hits": 2419,
  "cache_misses": 2593,
  "hit_rate": 0.482642,
  "dram_hits": 1834,
  "flash_hits": 585,
  "flash_bytes_written": 2097152,
  "logical_bytes_admitted": 1636288,
  "write_amplification": 1.281656,
  "segment_flushes": 2,
  "evictions": 0
}
```

Exact numbers depend on the trace and configuration.

## Interpreting Write Amplification

The simulator reports:

```text
write amplification = flash_bytes_written / logical_bytes_admitted
```

`logical_bytes_admitted` is the cumulative byte size of objects accepted into the
flash layer. `flash_bytes_written` is the physical segment write cost charged by
the simulator. Because v0.1 writes full segments, small traces or partially filled
final segments can show high write amplification.

To compare admission behavior across policies, also compare `flash_bytes_written`.
On update-heavy workloads, `flashield-lite` should write substantially fewer flash
bytes than `naive-flash` because unstable objects remain in DRAM or are discarded
instead of immediately being written to flash.

## Next Steps

- Add trace streaming for very large files.
- Add richer admission features such as inter-arrival time and object popularity
  windows.
- Add configurable workload generators for read-heavy, update-heavy, and skewed
  Zipf-like access patterns.
- Model flash invalidation, cleaning, and erase-block-level amplification.
- Export reports as JSON or CSV for plotting.
