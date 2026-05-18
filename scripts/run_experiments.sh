#!/usr/bin/env bash
set -euo pipefail

output_dir="${OUTPUT_DIR:-results}"
requests="${REQUESTS:-10000}"
dram_capacity="${DRAM_CAPACITY:-1048576}"
flash_capacity="${FLASH_CAPACITY:-10485760}"
segment_size="${SEGMENT_SIZE:-1048576}"
trace_dir="traces/generated"

presets=("mixed" "read-heavy" "update-heavy")
policies=("dram-lru" "naive-flash" "flashield-lite")

mkdir -p "$output_dir" "$trace_dir"

for preset in "${presets[@]}"; do
    trace_path="$trace_dir/$preset.csv"

    cargo run --quiet -- generate-trace \
        --output "$trace_path" \
        --requests "$requests" \
        --preset "$preset"

    for policy in "${policies[@]}"; do
        report_path="$output_dir/$preset-$policy.json"

        cargo run --quiet -- simulate \
            --policy "$policy" \
            --trace "$trace_path" \
            --dram-capacity "$dram_capacity" \
            --flash-capacity "$flash_capacity" \
            --segment-size "$segment_size" \
            --output-format json > "$report_path"

        echo "wrote $report_path"
    done
done
