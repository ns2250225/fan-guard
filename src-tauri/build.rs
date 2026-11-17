use std::path::Path;

fn main() {
    // 若传感器 Sidecar 已构建，则将其嵌入主程序，发布为单一 exe
    let sidecar = Path::new("binaries/fanguard-sensors-x86_64-pc-windows-msvc.exe");
    println!("cargo::rustc-check-cfg=cfg(embedded_sidecar)");
    println!("cargo:rerun-if-changed=binaries/fanguard-sensors-x86_64-pc-windows-msvc.exe");
    println!("cargo:rerun-if-changed=icons/icon.ico");
    if let Ok(bytes) = std::fs::read(sidecar) {
        // FNV-1a 64 位哈希，用于区分 Sidecar 版本
        let mut h: u64 = 0xcbf29ce484222325;
        for b in &bytes {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        println!("cargo:rustc-cfg=embedded_sidecar");
        println!("cargo:rustc-env=FANGUARD_SIDECAR_HASH={:016x}", h);
    } else {
        println!("cargo:warning=未找到传感器 Sidecar，温度与风扇数据将不可用。请先运行 npm run sidecar");
    }
    tauri_build::build()
}
