#!/usr/bin/env python3
"""Summarize flashield-lite JSON reports as a CSV table."""

from __future__ import annotations

import argparse
import csv
import json
from pathlib import Path


FIELDS = [
    "preset",
    "policy",
    "total_requests",
    "lookup_requests",
    "cache_hits",
    "cache_misses",
    "hit_rate",
    "dram_hits",
    "flash_hits",
    "flash_bytes_written",
    "logical_bytes_admitted",
    "write_amplification",
    "segment_flushes",
    "evictions",
]


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Summarize flashield-lite JSON result files as CSV."
    )
    parser.add_argument(
        "--input-dir",
        default="results",
        type=Path,
        help="Directory containing JSON reports, default: results",
    )
    parser.add_argument(
        "--output",
        default=Path("results") / "summary.csv",
        type=Path,
        help="CSV summary path, default: results/summary.csv",
    )
    args = parser.parse_args()

    rows = list(load_rows(args.input_dir))
    if not rows:
        raise SystemExit(f"no JSON reports found in {args.input_dir}")

    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=FIELDS)
        writer.writeheader()
        writer.writerows(rows)

    print(f"wrote {args.output}")


def load_rows(input_dir: Path) -> list[dict[str, object]]:
    rows = []
    for path in sorted(input_dir.glob("*.json")):
        with path.open(encoding="utf-8") as handle:
            report = json.load(handle)
        preset = infer_preset(path, report)
        row = {field: report.get(field, "") for field in FIELDS}
        row["preset"] = preset
        row["policy"] = report.get("policy", "")
        rows.append(row)
    return rows


def infer_preset(path: Path, report: dict[str, object]) -> str:
    policy = str(report.get("policy", ""))
    stem = path.stem
    suffix = f"-{policy}"
    if policy and stem.endswith(suffix):
        return stem[: -len(suffix)]
    return stem


if __name__ == "__main__":
    main()
