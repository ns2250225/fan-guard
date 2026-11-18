# 构建 FanGuard 单文件 exe（无安装包、无控制台黑框）
#
# 内置内容：
#   - 传感器 Sidecar（LibreHardwareMonitor）       src-tauri/binaries/fanguard-sensors-*.exe（缺失时自动构建）
#   - PawnIO 驱动安装程序（作者允许原样再分发）       src-tauri/binaries/PawnIO_setup.exe（缺失时自动下载并校验签名）
#   - HWiNFO64.exe（从项目根目录的 hwinfo.zip 提取） hwinfo.zip 需自行放置；不存在时不内置
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$bin = Join-Path $root "src-tauri\binaries"
New-Item -ItemType Directory -Force $bin | Out-Null

$sidecar = Join-Path $bin "fanguard-sensors-x86_64-pc-windows-msvc.exe"
if (-not (Test-Path $sidecar)) {
    Write-Host "== 构建传感器 Sidecar =="
    & powershell -ExecutionPolicy Bypass -File (Join-Path $PSScriptRoot "build-sidecar.ps1")
    if ($LASTEXITCODE -ne 0) { throw "Sidecar 构建失败" }
}

$pawnio = Join-Path $bin "PawnIO_setup.exe"
if (-not (Test-Path $pawnio)) {
    Write-Host "== 下载 PawnIO 安装程序 =="
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        $tmp = "$pawnio.download"
        Invoke-WebRequest -UseBasicParsing -Uri "https://github.com/namazso/PawnIO.Setup/releases/latest/download/PawnIO_setup.exe" -OutFile $tmp -TimeoutSec 120
        $sig = Get-AuthenticodeSignature -LiteralPath $tmp
        if ($sig.Status -ne "Valid" -or $sig.SignerCertificate.Subject -notmatch "namazso") {
            Remove-Item $tmp -Force
            throw "签名校验失败：$($sig.Status) $($sig.SignerCertificate.Subject)"
        }
        Move-Item $tmp $pawnio -Force
    } catch {
        Write-Warning "未能下载 PawnIO 安装程序（$_），将不内置；运行时会改为在线下载"
    }
}

$zip = Join-Path $root "hwinfo.zip"
if (-not (Test-Path $zip) -and -not (Test-Path (Join-Path $bin "hwinfo.zip"))) {
    Write-Warning "未找到 hwinfo.zip，本次构建不会内置 HWiNFO"
}
# 强制重新运行 build.rs，确保新放入或替换的 hwinfo.zip / PawnIO 被重新嵌入
(Get-Item (Join-Path $root "src-tauri\build.rs")).LastWriteTime = Get-Date

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
