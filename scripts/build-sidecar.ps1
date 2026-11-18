# 构建 LibreHardwareMonitor 传感器 Sidecar，并复制到 Tauri externalBin 目录
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$proj = Join-Path $root "sidecar\FanGuard.Sensors\FanGuard.Sensors.csproj"
$out = Join-Path $root "sidecar\publish"
$binDir = Join-Path $root "src-tauri\binaries"

dotnet publish $proj -c Release -r win-x64 --self-contained true -p:PublishSingleFile=true -o $out
if ($LASTEXITCODE -ne 0) { throw "dotnet publish failed" }

New-Item -ItemType Directory -Force $binDir | Out-Null
Copy-Item (Join-Path $out "fanguard-sensors.exe") (Join-Path $binDir "fanguard-sensors-x86_64-pc-windows-msvc.exe") -Force
Write-Host "Sidecar 已生成: $binDir\fanguard-sensors-x86_64-pc-windows-msvc.exe"
