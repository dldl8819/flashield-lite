#!/usr/bin/env python3
"""Render experiment summary CSV files as Markdown tables."""

from __future__ import annotations

import argparse
import csv
from pathlib import Path


DEFAULT_COLUMNS = [
    "preset",
    "policy",
    "hit_rate",
    "flash_bytes_written",
    "logical_bytes_admitted",
    "write_amplification",
    "flash_hits",
    "evictions",
]

HEADERS = {
    "preset": "Preset",
    "policy": "Policy",
    "hit_rate": "Hit rate",
    "flash_bytes_written": "Flash bytes written",
    "logical_bytes_admitted": "Logical bytes admitted",
    "write_amplification": "Write amplification",
    "flash_hits": "Flash hits",
    "evictions": "Evictions",
}


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Convert a flashield-lite experiment summary CSV to Markdown."
    )
    parser.add_argument(
        "--input",
        default=Path("results") / "summary.csv",
        type=Path,
        help="CSV summary path, default: results/summary.csv",
    )
    parser.add_argument(
        "--output",
        type=Path,
        help="Markdown output path. When omitted, print to stdout.",
    )
    args = parser.parse_args()

    rows = load_rows(args.input)
    if not rows:
        raise SystemExit(f"no rows found in {args.input}")

    markdown = render_markdown(rows, DEFAULT_COLUMNS)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(markdown, encoding="utf-8")
        print(f"wrote {args.output}")
    else:
        print(markdown, end="")


def load_rows(path: Path) -> list[dict[str, str]]:
    with path.open(newline="", encoding="utf-8-sig") as handle:
        return list(csv.DictReader(handle))


def render_markdown(rows: list[dict[str, str]], columns: list[str]) -> str:
    header = "| " + " | ".join(HEADERS[column] for column in columns) + " |"
    separator = "| " + " | ".join("---" for _ in columns) + " |"
    body = [
        "| " + " | ".join(format_cell(row.get(column, ""), column) for column in columns) + " |"
        for row in rows
    ]
    return "\n".join([header, separator, *body]) + "\n"


def format_cell(value: str, column: str) -> str:
    if value == "":
        return ""
    if column == "hit_rate":
        return format_float(value, multiplier=100.0, suffix="%")
    if column == "write_amplification":
        return format_float(value)
    if column in {
        "flash_bytes_written",
        "logical_bytes_admitted",
        "flash_hits",
        "evictions",
    }:
        return format_int(value)
    return value


def format_float(value: str, multiplier: float = 1.0, suffix: str = "") -> str:
    try:
        number = float(value) * multiplier
    except ValueError:
        return value
    return f"{number:.2f}{suffix}"


def format_int(value: str) -> str:
    try:
        number = int(float(value))
    except ValueError:
        return value
    return f"{number:,}"


if __name__ == "__main__":
    main()
