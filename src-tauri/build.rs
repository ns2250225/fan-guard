use std::path::{Path, PathBuf};

/// FNV-1a 64 位哈希，用于区分内嵌文件版本
fn fnv(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", h)
}

fn find_file(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut dirs = vec![];
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            dirs.push(p);
        } else if e.file_name().to_string_lossy().eq_ignore_ascii_case(name) {
            return Some(p);
        }
    }
    if depth == 0 {
        return None;
    }
    dirs.into_iter().find_map(|d| find_file(&d, name, depth - 1))
}

fn mtime(p: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// 从 HWiNFO 便携版压缩包中解出 HWiNFO64.exe 到 OUT_DIR
fn extract_hwinfo(zip: &Path) -> Option<PathBuf> {
    let out = PathBuf::from(std::env::var("OUT_DIR").ok()?).join("hwinfo");
    let exe = out.join("HWiNFO64.exe");
    if exe.exists() && mtime(&exe) >= mtime(zip) {
        return Some(exe);
    }
    let tmp = out.with_file_name("hwinfo_unzip");
    let _ = std::fs::remove_dir_all(&tmp);
    let script = format!(
        "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
        zip.display().to_string().replace('\'', "''"),
        tmp.display().to_string().replace('\'', "''")
    );
    let ok = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        println!("cargo:warning=解压 {} 失败，HWiNFO 将不会内置", zip.display());
        return None;
    }
    let found = match find_file(&tmp, "HWiNFO64.exe", 3) {
        Some(f) => f,
        None => {
            println!("cargo:warning={} 中没有 HWiNFO64.exe（需要便携版），HWiNFO 将不会内置", zip.display());
            let _ = std::fs::remove_dir_all(&tmp);
            return None;
        }
    };
    std::fs::create_dir_all(&out).ok()?;
    std::fs::copy(&found, &exe).ok()?;
    let _ = std::fs::remove_dir_all(&tmp);
    Some(exe)
}

fn main() {
    println!("cargo::rustc-check-cfg=cfg(embedded_sidecar)");
    println!("cargo::rustc-check-cfg=cfg(embedded_hwinfo)");
    println!("cargo::rustc-check-cfg=cfg(embedded_pawnio)");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=icons/icon.ico");

    // 1. 传感器 Sidecar（LibreHardwareMonitor）
    let sidecar = Path::new("binaries/fanguard-sensors-x86_64-pc-windows-msvc.exe");
    println!("cargo:rerun-if-changed={}", sidecar.display());
    if let Ok(bytes) = std::fs::read(sidecar) {
        println!("cargo:rustc-cfg=embedded_sidecar");
        println!("cargo:rustc-env=FANGUARD_SIDECAR_HASH={}", fnv(&bytes));
    } else {
        println!("cargo:warning=未找到传感器 Sidecar，温度与风扇数据将不可用。请先运行 npm run sidecar");
    }

    // 2. HWiNFO：项目根目录或 binaries 目录下的 hwinfo.zip（便携版）
    let zips = [Path::new("../hwinfo.zip"), Path::new("binaries/hwinfo.zip")];
    for z in zips.iter().filter(|z| z.exists()) {
        println!("cargo:rerun-if-changed={}", z.display());
    }
    match zips.iter().find(|z| z.exists()).and_then(|z| extract_hwinfo(z)) {
        Some(exe) => {
            let bytes = std::fs::read(&exe).unwrap_or_default();
            println!("cargo:rustc-cfg=embedded_hwinfo");
            println!("cargo:rustc-env=FANGUARD_HWINFO_EXE={}", exe.display());
            println!("cargo:rustc-env=FANGUARD_HWINFO_HASH={}", fnv(&bytes));
        }
        None => println!("cargo:warning=未找到 hwinfo.zip，HWiNFO 不会内置（运行时仍可导入）"),
    }

    // 3. PawnIO 驱动安装程序（作者声明可原样再分发）
    let pawnio = Path::new("binaries/PawnIO_setup.exe");
    println!("cargo:rerun-if-changed={}", pawnio.display());
    if pawnio.exists() {
        println!("cargo:rustc-cfg=embedded_pawnio");
    } else {
        println!("cargo:warning=未找到 PawnIO_setup.exe，安装 PawnIO 时将从 GitHub 下载");
    }

    tauri_build::build()
}
