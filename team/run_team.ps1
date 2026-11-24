param(
    [Parameter(Position = 0)]
    [string]$Task = "",
    [switch]$Verify,
    [switch]$WeeklyReview
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$SkillsDir = Join-Path $Root ".agents\skills"
$MemoryFile = Join-Path $PSScriptRoot "team_memory.md"
$LogFile = Join-Path $PSScriptRoot "runs_log.jsonl"

$RequiredSkills = @("team-lead", "team-scout", "team-architect", "team-builder", "team-checker")
Write-Host "=== Feature & Quality AI Team — Runner v3 ===" -ForegroundColor Cyan

foreach ($skill in $RequiredSkills) {
    $path = Join-Path $SkillsDir "$skill\SKILL.md"
    if (-not (Test-Path $path)) {
        Write-Error "Skill mancante: $path"
    }
    Write-Host "  [OK] Skill pronta: $skill" -ForegroundColor Green
}

if (-not (Test-Path $MemoryFile)) {
    Write-Error "team_memory.md mancante: $MemoryFile"
}
Write-Host "  [OK] Team memory: $MemoryFile" -ForegroundColor Green

if ($Verify) {
    Write-Host "`n=== Checker Independent Verification (cargo test --workspace) ===" -ForegroundColor Cyan
    Push-Location $Root
    try {
        cargo test --workspace
        if ($LASTEXITCODE -ne 0) {
            Write-Error "Verification FAILED (cargo test exit code $LASTEXITCODE)"
        }
        Write-Host "  [PASS] Workspace tests passed." -ForegroundColor Green
    } finally {
        Pop-Location
    }
}

if (Test-Path $LogFile) {
    $lines = Get-Content $LogFile | Where-Object { $_.Trim() -ne "" }
    $total = $lines.Count
    $firstPass = 0
    $weakCounts = @{}
    $scores = @()

    foreach ($line in $lines) {
        $obj = $line | ConvertFrom-Json
        if ($obj.first_pass_approved -eq $true) { $firstPass++ }
        if ($null -ne $obj.checker_score) { $scores += [int]$obj.checker_score }
        if ($null -ne $obj.weakest_agent -and $obj.weakest_agent -ne "") {
            $w = [string]$obj.weakest_agent
            if (-not $weakCounts.ContainsKey($w)) { $weakCounts[$w] = 0 }
            $weakCounts[$w]++
        }
    }

    $rate = if ($total -gt 0) { [math]::Round(($firstPass / $total) * 100, 1) } else { 0 }
    $avgScore = if ($scores.Count -gt 0) { [math]::Round(($scores | Measure-Object -Average).Average, 1) } else { 0 }
    Write-Host "`n  [KPI] Run: $total | First-pass rate: $rate% | Avg Checker score: $avgScore/100" -ForegroundColor Yellow

    if ($WeeklyReview) {
        Write-Host "`n=== Weekly Improvement Loop Summary ===" -ForegroundColor Magenta
        if ($weakCounts.Count -eq 0) {
            Write-Host "  Nessun agente debole segnalato nei log: nessuna modifica prompt necessaria." -ForegroundColor Green
        } else {
            $topWeak = $weakCounts.GetEnumerator() | Sort-Object Value -Descending | Select-Object -First 1
            Write-Host "  Agente piu debole: $($topWeak.Key) ($($topWeak.Value) FAIL)" -ForegroundColor Yellow
            Write-Host "  Prossimo passo: il Lead propone UNA sola modifica al prompt di $($topWeak.Key) e attende il tuo SI."
        }
    }
} else {
    Write-Host "  [KPI] Nessuna run ancora registrata." -ForegroundColor Yellow
}

if ($Task -ne "") {
    Write-Host "`n=== Prompt pronto per Antigravity (Lead) ===" -ForegroundColor Cyan
    Write-Host "Attiva la skill team-lead. Leggi team/team_memory.md ed esegui l'intero grafo (Scout + Architect in parallelo -> Join -> Builder -> Checker max 2 round -> UserApproval) per questo task: $Task"
}
