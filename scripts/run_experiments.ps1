param(
    [string]$OutputDir = "results",
    [int]$Requests = 10000,
    [int]$DramCapacity = 1048576,
    [int]$FlashCapacity = 10485760,
    [int]$SegmentSize = 1048576,
    [string]$Python = "python"
)

$ErrorActionPreference = "Stop"

$presets = @("mixed", "read-heavy", "update-heavy")
$policies = @("dram-lru", "naive-flash", "flashield-lite")
$traceDir = Join-Path "traces" "generated"

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
New-Item -ItemType Directory -Force -Path $traceDir | Out-Null

foreach ($preset in $presets) {
    $tracePath = Join-Path $traceDir "$preset.csv"

    cargo run --quiet -- generate-trace `
        --output $tracePath `
        --requests $Requests `
        --preset $preset

    foreach ($policy in $policies) {
        $reportPath = Join-Path $OutputDir "$preset-$policy.json"

        cargo run --quiet -- simulate `
            --policy $policy `
            --trace $tracePath `
            --dram-capacity $DramCapacity `
            --flash-capacity $FlashCapacity `
            --segment-size $SegmentSize `
            --output-format json |
            Set-Content -NoNewline -Encoding UTF8 $reportPath

        Write-Host "wrote $reportPath"
    }
}

$summaryPath = Join-Path $OutputDir "summary.csv"
& $Python scripts/summarize_results.py --input-dir $OutputDir --output $summaryPath
