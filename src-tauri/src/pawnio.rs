//! PawnIO 驱动检测与安装
//!
//! LibreHardwareMonitor 0.9.5 起通过 PawnIO 驱动访问 MSR / Super I/O / EC 等底层硬件：
//! 未安装时，即使以管理员身份运行也读不到 Intel/AMD CPU 温度与功耗、台式机主板风扇转速。
//!
//! - 检测：与 LibreHardwareMonitor 相同，读取 HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO 的 DisplayVersion；
//! - 安装：使用内置的官方安装程序（作者声明「可原样再分发」），没有内置时从官方 GitHub Release 下载；
//!   运行前校验数字签名（namazso），通过一次 UAC 授权执行 `-install -silent`。

use crate::util::{wide, CREATE_NO_WINDOW};
use serde::Serialize;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};

const UNINSTALL_KEY: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\PawnIO";
const DOWNLOAD_URL: &str = "https://github.com/namazso/PawnIO.Setup/releases/latest/download/PawnIO_setup.exe";
const HOMEPAGE: &str = "https://pawnio.eu/";
const ERROR_CANCELLED: u32 = 1223;

#[cfg(embedded_pawnio)]
static EMBEDDED: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/binaries/PawnIO_setup.exe"));
#[cfg(not(embedded_pawnio))]
static EMBEDDED: &[u8] = &[];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PawnioStatus {
    pub installed: bool,
    pub version: Option<String>,
    /// 内置安装程序的版本（未内置时为 None，安装时从网络下载）
    pub bundled_version: Option<String>,
    /// 已安装版本低于内置版本
    pub update_available: bool,
    pub installing: bool,
}

struct Cache {
    at: Option<Instant>,
    status: PawnioStatus,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    at: None,
    status: PawnioStatus {
        installed: false,
        version: None,
        bundled_version: None,
        update_available: false,
        installing: false,
    },
});

static BUNDLED_VERSION: Mutex<Option<Option<String>>> = Mutex::new(None);

fn parse_ver(s: &str) -> Vec<u32> {
    s.split(|c: char| !c.is_ascii_digit()).filter(|x| !x.is_empty()).filter_map(|x| x.parse().ok()).collect()
}

fn older(a: &str, b: &str) -> bool {
    let (mut x, mut y) = (parse_ver(a), parse_ver(b));
    let n = x.len().max(y.len());
    x.resize(n, 0);
    y.resize(n, 0);
    x < y
}

fn installer_dir() -> PathBuf {
    std::env::temp_dir().join("FanGuard-PawnIO")
}

/// 将内置安装程序写到临时目录；返回路径
fn write_embedded() -> Option<PathBuf> {
    if EMBEDDED.is_empty() {
        return None;
    }
    let dir = installer_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let p = dir.join("PawnIO_setup.exe");
    let same = std::fs::metadata(&p).map(|m| m.len() == EMBEDDED.len() as u64).unwrap_or(false);
    if !same {
        std::fs::write(&p, EMBEDDED).ok()?;
    }
    Some(p)
}

fn bundled_version() -> Option<String> {
    let mut g = BUNDLED_VERSION.lock().unwrap();
    if let Some(v) = g.as_ref() {
        return v.clone();
    }
    let v = write_embedded().and_then(|p| crate::process_ops::file_info(&p.to_string_lossy()).version);
    *g = Some(v.clone());
    v
}

fn read_installed() -> Option<String> {
    crate::hardware::reg_string(UNINSTALL_KEY, "DisplayVersion")
        .or_else(|| crate::hardware::reg_string("SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\PawnIO", "DisplayVersion"))
}

/// 当前状态（缓存 10 秒）
pub fn status() -> PawnioStatus {
    {
        let c = CACHE.lock().unwrap();
        if c.at.map_or(false, |t| t.elapsed() < Duration::from_secs(10)) {
            return c.status.clone();
        }
    }
    refresh()
}

pub fn refresh() -> PawnioStatus {
    let version = read_installed();
    let bundled = bundled_version();
    let mut c = CACHE.lock().unwrap();
    let st = PawnioStatus {
        installed: version.is_some(),
        update_available: match (&version, &bundled) {
            (Some(v), Some(b)) => older(v, b),
            _ => false,
        },
        version,
        bundled_version: bundled,
        installing: c.status.installing,
    };
    c.status = st.clone();
    c.at = Some(Instant::now());
    st
}

fn set_installing(v: bool) {
    let mut c = CACHE.lock().unwrap();
    c.status.installing = v;
}

fn download() -> Result<PathBuf, String> {
    let dir = installer_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let p = dir.join("PawnIO_setup_download.exe");
    let _ = std::fs::remove_file(&p);
    let script = format!(
        "[Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12; Invoke-WebRequest -UseBasicParsing -Uri '{}' -OutFile '{}' -TimeoutSec 120",
        DOWNLOAD_URL,
        p.display().to_string().replace('\'', "''")
    );
    let ok = std::process::Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok && p.exists() {
        Ok(p)
    } else {
        Err("下载 PawnIO 安装程序失败，请检查网络，或点击「打开官网」手动安装".into())
    }
}

/// 以管理员身份运行并等待结束；返回进程退出码
fn run_elevated_wait(exe: &Path, params: &str) -> Result<u32, u32> {
    let verb = wide("runas");
    let file = wide(&exe.to_string_lossy());
    let params = wide(params);
    unsafe {
        use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        let mut sei: SHELLEXECUTEINFOW = std::mem::zeroed();
        sei.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        sei.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
        sei.lpVerb = verb.as_ptr();
        sei.lpFile = file.as_ptr();
        sei.lpParameters = params.as_ptr();
        sei.nShow = 0;
        if ShellExecuteExW(&mut sei) == 0 {
            return Err(GetLastError());
        }
        if sei.hProcess.is_null() {
            return Ok(0);
        }
        let mut code = 0u32;
        if WaitForSingleObject(sei.hProcess, 180_000) == WAIT_OBJECT_0 {
            GetExitCodeProcess(sei.hProcess, &mut code);
        }
        CloseHandle(sei.hProcess);
        Ok(code)
    }
}

/// 安装或更新 PawnIO；成功返回安装后的版本
pub fn install() -> Result<String, String> {
    set_installing(true);
    let r = install_inner();
    set_installing(false);
    refresh();
    r
}

fn install_inner() -> Result<String, String> {
    let exe = match write_embedded() {
        Some(p) => p,
        None => download()?,
    };
    // 安装驱动需要管理员权限：必须确认是作者签名的官方安装程序
    let sig = crate::process_ops::signature(&exe.to_string_lossy());
    let signer = sig.signer.clone().unwrap_or_default();
    if sig.status != "valid" || !signer.to_lowercase().contains("namazso") {
        let _ = std::fs::remove_file(&exe);
        return Err(format!(
            "PawnIO 安装程序数字签名校验未通过（{}，签名者：{}），已取消安装",
            sig.status_text,
            if signer.is_empty() { "无" } else { &signer }
        ));
    }
    match run_elevated_wait(&exe, "-install -silent") {
        Ok(code) => {
            let st = refresh();
            match (st.installed, code) {
                (true, _) => Ok(st.version.unwrap_or_default()),
                (false, 0) => Err("安装程序已运行，但未检测到 PawnIO，请重启电脑后再试".into()),
                (false, c) => Err(format!("PawnIO 安装失败（退出码 {}）", c)),
            }
        }
        Err(ERROR_CANCELLED) => Err("已取消管理员授权，PawnIO 未安装".into()),
        Err(c) => Err(format!("无法启动 PawnIO 安装程序（错误码 {}）", c)),
    }
}

pub fn open_homepage() -> Result<(), String> {
    let r = unsafe {
        windows_sys::Win32::UI::Shell::ShellExecuteW(
            std::ptr::null_mut(),
            wide("open").as_ptr(),
            wide(HOMEPAGE).as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    };
    if (r as isize) > 32 {
        Ok(())
    } else {
        Err("无法打开浏览器".into())
    }
}
