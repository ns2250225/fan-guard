# 构建 FanGuard 单文件 exe（无安装包、无控制台黑框）
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$sidecar = Join-Path $root "src-tauri\binaries\fanguard-sensors-x86_64-pc-windows-msvc.exe"
if (-not (Test-Path $sidecar)) {
    Write-Host "== 构建传感器 Sidecar =="
    & powershell -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "build-sidecar.ps1")
    if ($LASTEXITCODE -ne 0) { throw "Sidecar 构建失败" }
}

Write-Host "== 绘制程序图标 =="
node scripts/gen-icon.mjs
if ($LASTEXITCODE -ne 0) { throw "图标生成失败" }

Write-Host "== 编译 FanGuard（release，不打包安装程序）=="
npx tauri build --no-bundle
if ($LASTEXITCODE -ne 0) { throw "编译失败" }

$out = Join-Path $root "release"
New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item (Join-Path $root "src-tauri\target\release\fanguard.exe") (Join-Path $out "FanGuard.exe") -Force
$size = [math]::Round((Get-Item (Join-Path $out "FanGuard.exe")).Length / 1MB, 1)
Write-Host "完成：$out\FanGuard.exe ($size MB)"
