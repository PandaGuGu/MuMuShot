# MuMuShot 性能评测：图形面板常驻资源对比（Python 版 vs Rust 版）
# 用法： powershell -ExecutionPolicy Bypass -File bench\mem.ps1

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)
$PyExe = Join-Path $Root "MuMuShot.exe"
$RsExe = Join-Path $Root "rust\target\release\MuMuShot.exe"

function Kill-ByName($name) {
    Get-Process -Name $name -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
}

function Measure-Panel($exe, $procName, $label, $settleSeconds) {
    Kill-ByName $procName
    Start-Sleep -Milliseconds 400

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $p = Start-Process -FilePath $exe -PassThru

    # 面板可见（主窗口出现）耗时
    $ready = -1
    for ($i = 0; $i -lt 400; $i++) {
        Start-Sleep -Milliseconds 20
        if ($p.HasExited) { break }
        $p.Refresh()
        if ($p.MainWindowHandle -ne 0) { $ready = $sw.ElapsedMilliseconds; break }
    }

    Start-Sleep -Seconds $settleSeconds
    $p.Refresh()

    $result = [ordered]@{
        impl            = $label
        ready_ms        = $ready
        ws_mb           = [math]::Round($p.WorkingSet64 / 1MB, 1)
        private_mb      = [math]::Round($p.PrivateMemorySize64 / 1MB, 1)
        peak_ws_mb      = [math]::Round($p.PeakWorkingSet64 / 1MB, 1)
        threads         = $p.Threads.Count
        handles         = $p.HandleCount
    }

    Stop-Process -Id $p.Id -Force -ErrorAction SilentlyContinue
    Kill-ByName $procName
    Start-Sleep -Milliseconds 600
    return [pscustomobject]$result
}

$py = Measure-Panel $PyExe "MuMuShot" "Python" 5
$rs = Measure-Panel $RsExe "MuMuShot" "Rust" 5

$py, $rs | Format-Table -AutoSize

$outDir = Join-Path $Root "bench\results"
New-Item -ItemType Directory -Force -Path $outDir | Out-Null
$py, $rs | ConvertTo-Json | Set-Content -Encoding UTF8 (Join-Path $outDir "mem.json")
Write-Host "已写入 bench\results\mem.json"
