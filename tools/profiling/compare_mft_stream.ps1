param(
    [Parameter(Mandatory)][string]$Baseline,
    [Parameter(Mandatory)][string]$Candidate,
    [ValidateRange(2, 1000)][int]$Rounds = 8,
    [ValidateRange(1, 100000)][int]$Iterations = 30,
    [string]$OutputPath = 'target/performance-audit/paired.csv'
)

$ErrorActionPreference = 'Stop'
$executables = @{
    baseline = (Resolve-Path -LiteralPath $Baseline).Path
    candidate = (Resolve-Path -LiteralPath $Candidate).Path
}
$rows = [System.Collections.Generic.List[object]]::new()
foreach ($source in @('memory', 'file')) {
    foreach ($mode in @('records', 'extensions')) {
        $expectedFingerprint = $null
        for ($round = 0; $round -lt $Rounds; $round++) {
            $order = if ($round % 2 -eq 0) { @('baseline', 'candidate') } else { @('candidate', 'baseline') }
            foreach ($variant in $order) {
                $output = & $executables[$variant] $source $mode $Iterations
                if ($LASTEXITCODE -ne 0) { throw "$variant failed for $source/$mode" }
                $line = $output -join "`n"
                if ($line -notmatch 'iterations=(\d+) ns=(\d+) records=(\d+) errors=(\d+) fingerprint=([0-9a-f]+)$') {
                    throw "Unexpected profile output: $line"
                }
                if ([int]$Matches[1] -ne $Iterations -or [int]$Matches[3] -ne 13068 -or [int]$Matches[4] -ne 20) {
                    throw "Workload mismatch: $line"
                }
                $fingerprint = $Matches[5]
                if ($null -eq $expectedFingerprint) { $expectedFingerprint = $fingerprint }
                if ($fingerprint -ne $expectedFingerprint) { throw "Result fingerprint changed: $line" }
                $rows.Add([pscustomobject]@{
                    source = $source
                    mode = $mode
                    round = $round + 1
                    variant = $variant
                    iterations = $Iterations
                    nanoseconds = [long]$Matches[2]
                    records = 13068
                    errors = 20
                    fingerprint = $fingerprint
                })
            }
        }
        Write-Output "Completed $Rounds alternating pairs for $source/$mode"
    }
}
$parent = Split-Path -Parent $OutputPath
if ($parent) { New-Item -ItemType Directory -Force -Path $parent | Out-Null }
$rows | Export-Csv -LiteralPath $OutputPath -NoTypeInformation
Write-Output "Saved $($rows.Count) measurements to $OutputPath"
