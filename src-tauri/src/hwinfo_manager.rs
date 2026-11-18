//! HWiNFO 自动集成
//!
//! 负责在不需要用户手动操作的前提下，让 HWiNFO 为 FanGuard 提供传感器数据：
//! 1. 自动查找：正在运行的实例、FanGuard 托管目录、程序同目录、注册表安装位置、常见安装路径；
//! 2. 自动导入：在「下载」「桌面」文件夹中发现 HWiNFO 便携版压缩包（hwi_*.zip）或 HWiNFO64.exe 时，
//!    校验数字签名（REALiX）后解压到 %LOCALAPPDATA%\FanGuard\hwinfo；
//! 3. 自动配置：写入 HWiNFO64.INI —— 仅传感器模式、开启共享内存、启动时最小化、不显示欢迎窗口；
//! 4. 自动启动：LibreHardwareMonitor 读取不到 CPU 温度 / 风扇时启动 HWiNFO
//!    （HWiNFO 需要管理员权限，FanGuard 未提权时会出现一次 UAC 确认）；
//! 5. 自动续期：免费版共享内存 12 小时后失效，检测到后自动重启 HWiNFO。
//!
//! 构建时若项目中存在 hwinfo.zip（HWiNFO 便携版），会将其中的 HWiNFO64.exe 内置到 FanGuard，
//! 首次需要时释放到托管目录，用户无需自行下载。每次以管理员权限启动前都会校验 REALiX 数字签名。

use crate::process::{query_image_path, query_processes};
use crate::process_ops;
use crate::sensors::{SensorHub, SRC_HWINFO, SRC_LHM};
use crate::util::{now_ms, wide, CREATE_NO_WINDOW};
use serde::Serialize;
use std::collections::HashSet;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};

const EXE_NAMES: &[&str] = &["HWiNFO64.exe", "HWiNFO32.exe", "HWiNFO.exe"];

/// 构建时从 hwinfo.zip 中提取并内置的 HWiNFO64.exe
#[cfg(embedded_hwinfo)]
static EMBEDDED: &[u8] = include_bytes!(env!("FANGUARD_HWINFO_EXE"));
#[cfg(embedded_hwinfo)]
const EMBEDDED_HASH: &str = env!("FANGUARD_HWINFO_HASH");
const BUNDLE_MARKER: &str = "fanguard-bundled.txt";

pub const BUNDLED: bool = cfg!(embedded_hwinfo);
const ERROR_CANCELLED: u32 = 1223;
const SW_HIDE: i32 = 0;
const SW_SHOWMINNOACTIVE: i32 = 7;

/// 写入 HWiNFO 配置文件 [Settings] 段的键值
const INI_SETTINGS: &[(&str, &str)] = &[
    ("SensorsOnly", "1"),
    ("SensorsSM", "1"),
    ("OpenSensors", "1"),
    ("MinimalizeSensors", "1"),
    ("MinimalizeMainWnd", "1"),
    ("ShowWelcomeAndProgress", "0"),
    ("AutoUpdate", "0"),
];

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HwinfoMgrStatus {
    /// 是否开启自动集成
    pub auto: bool,
    /// disabled / notInstalled / importing / ready / launching / running / declined / error
    pub stage: String,
    pub path: Option<String>,
    /// 是否为 FanGuard 托管的副本
    pub managed: bool,
    /// 是否为 FanGuard 内置的 HWiNFO
    pub bundled: bool,
    pub running: bool,
    pub launched_by_us: bool,
    /// 配置文件是否已写入（安装在 Program Files 且未提权时可能无法写入）
    pub configured: Option<bool>,
    pub message: Option<String>,
    pub updated_at: i64,
}

pub struct HwinfoManager {
    hub: Arc<SensorHub>,
    status: Mutex<HwinfoMgrStatus>,
    auto: AtomicBool,
    declined: AtomicBool,
    force_launch: AtomicBool,
    launched_by_us: AtomicBool,
    kick: (Mutex<bool>, Condvar),
    started: Instant,
    last_launch: Mutex<Option<Instant>>,
    last_restart: Mutex<Option<Instant>>,
    rejected: Mutex<HashSet<PathBuf>>,
    /// 已通过签名校验的文件（路径、大小、修改时间），避免重复校验
    verified: Mutex<Option<(PathBuf, u64, Option<std::time::SystemTime>)>>,
}

/// 将内置的 HWiNFO 释放到托管目录（版本未变化时跳过）；返回 exe 路径
#[cfg(embedded_hwinfo)]
fn ensure_embedded() -> Option<PathBuf> {
    let dir = managed_dir();
    let exe = dir.join("HWiNFO64.exe");
    let marker = dir.join(BUNDLE_MARKER);
    let current = std::fs::read_to_string(&marker).unwrap_or_default();
    let size_ok = std::fs::metadata(&exe).map(|m| m.len() == EMBEDDED.len() as u64).unwrap_or(false);
    if current.trim() == EMBEDDED_HASH && size_ok {
        return Some(exe);
    }
    // 托管副本正在运行时无法替换，继续使用现有文件
    if find_running().map_or(false, |(_, p)| p.map_or(false, |p| p.starts_with(&dir))) {
        return exe.is_file().then_some(exe);
    }
    std::fs::create_dir_all(&dir).ok()?;
    let tmp = dir.join(format!("HWiNFO64.exe.{}.tmp", std::process::id()));
    std::fs::write(&tmp, EMBEDDED).ok()?;
    let _ = std::fs::remove_file(&exe);
    if std::fs::rename(&tmp, &exe).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return exe.is_file().then_some(exe);
    }
    let _ = std::fs::write(&marker, EMBEDDED_HASH);
    configure_ini(&exe);
    Some(exe)
}

#[cfg(not(embedded_hwinfo))]
fn ensure_embedded() -> Option<PathBuf> {
    None
}

/// 托管目录中的 HWiNFO 是否就是内置版本
fn is_bundled_copy(exe: &Path) -> bool {
    BUNDLED
        && exe.starts_with(managed_dir())
        && std::fs::read_to_string(managed_dir().join(BUNDLE_MARKER)).map_or(false, |s| !s.trim().is_empty())
}

fn managed_dir() -> PathBuf {
    std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir())
        .join("FanGuard")
        .join("hwinfo")
}

fn is_hwinfo_name(name: &str) -> bool {
    EXE_NAMES.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// 正在运行的 HWiNFO：(pid, 路径)
fn find_running() -> Option<(u32, Option<PathBuf>)> {
    query_processes(false, None)
        .into_iter()
        .find(|p| is_hwinfo_name(&p.name))
        .map(|p| (p.pid, query_image_path(p.pid).map(PathBuf::from)))
}

fn exe_in(dir: &Path) -> Option<PathBuf> {
    EXE_NAMES[..2].iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

fn find_installed() -> Option<PathBuf> {
    if let Some(p) = exe_in(&managed_dir()) {
        return Some(p);
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) {
        for d in [dir.clone(), dir.join("HWiNFO"), dir.join("HWiNFO64")] {
            if let Some(p) = exe_in(&d) {
                return Some(p);
            }
        }
    }
    let app_path = crate::hardware::reg_string(
        "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\App Paths\\HWiNFO64.EXE",
        "",
    )
    .map(|s| PathBuf::from(s.trim_matches('"')))
    .filter(|p| p.is_file());
    if app_path.is_some() {
        return app_path;
    }
    for key in ["HWiNFO64_is1", "HWiNFO_is1"] {
        if let Some(loc) = crate::hardware::reg_string(
            &format!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{}", key),
            "InstallLocation",
        ) {
            if let Some(p) = exe_in(Path::new(loc.trim_matches('"'))) {
                return Some(p);
            }
        }
    }
    for base in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        if let Ok(pf) = std::env::var(base) {
            for sub in ["HWiNFO64", "HWiNFO"] {
                if let Some(p) = exe_in(&Path::new(&pf).join(sub)) {
                    return Some(p);
                }
            }
        }
    }
    None
}

/// 在「下载」「桌面」及 FanGuard 所在目录中查找可导入的 HWiNFO 便携版（最新的一个）
fn find_download(rejected: &HashSet<PathBuf>) -> Option<PathBuf> {
    let home = std::env::var("USERPROFILE").ok()?;
    let mut dirs: Vec<PathBuf> = ["Downloads", "Desktop", "下载"].iter().map(|s| Path::new(&home).join(s)).collect();
    // FanGuard.exe 所在目录及其上一级（便携使用时用户常把压缩包放在一起）
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_path_buf())) {
        if let Some(parent) = dir.parent() {
            dirs.push(parent.to_path_buf());
        }
        dirs.push(dir);
    }
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for d in dirs {
        let rd = match std::fs::read_dir(&d) {
            Ok(r) => r,
            Err(_) => continue,
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_lowercase();
            let ok = (name.starts_with("hwi") && name.ends_with(".zip")) || name == "hwinfo64.exe";
            if !ok || rejected.contains(&e.path()) {
                continue;
            }
            let Ok(meta) = e.metadata() else { continue };
            let Ok(modified) = meta.modified() else { continue };
            // 文件可能仍在写入
            if modified.elapsed().map_or(true, |d| d < Duration::from_secs(5)) {
                continue;
            }
            if best.as_ref().map_or(true, |(t, _)| modified > *t) {
                best = Some((modified, e.path()));
            }
        }
    }
    best.map(|(_, p)| p)
}

fn find_file_recursive(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
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
    dirs.into_iter().find_map(|d| find_file_recursive(&d, name, depth - 1))
}

/// 以 Latin-1 方式处理 INI，保留非 ASCII 字节不变
fn merge_ini(bytes: &[u8], kv: &[(&str, &str)]) -> Vec<u8> {
    let text: String = bytes.iter().map(|&b| b as char).collect();
    let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
    let start = match lines.iter().position(|l| l.trim().eq_ignore_ascii_case("[Settings]")) {
        Some(i) => i,
        None => {
            if lines.last().map_or(false, |l| !l.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.push("[Settings]".into());
            lines.len() - 1
        }
    };
    let mut end = lines[start + 1..]
        .iter()
        .position(|l| l.trim_start().starts_with('['))
        .map(|p| start + 1 + p)
        .unwrap_or(lines.len());
    for (k, v) in kv {
        let found = (start + 1..end).find(|&i| {
            lines[i]
                .split('=')
                .next()
                .map_or(false, |x| x.trim().eq_ignore_ascii_case(k))
        });
        match found {
            Some(i) => lines[i] = format!("{}={}", k, v),
            None => {
                lines.insert(end, format!("{}={}", k, v));
                end += 1;
            }
        }
    }
    let out = lines.join("\r\n") + "\r\n";
    out.chars().map(|c| c as u32 as u8).collect()
}

/// 写入 HWiNFO 配置；返回是否成功
fn configure_ini(exe: &Path) -> bool {
    let stem = exe.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "HWiNFO64".into());
    let ini = exe.with_file_name(format!("{}.INI", stem));
    let old = std::fs::read(&ini).unwrap_or_default();
    let new = merge_ini(&old, INI_SETTINGS);
    if new == old {
        return true;
    }
    std::fs::write(&ini, new).is_ok()
}

/// ShellExecuteEx；失败时返回 GetLastError
fn shell_execute(verb: &str, file: &str, params: &str, dir: Option<&str>, show: i32) -> Result<(), u32> {
    let verb = wide(verb);
    let file = wide(file);
    let params = wide(params);
    let dir = dir.map(wide);
    unsafe {
        let mut sei: SHELLEXECUTEINFOW = std::mem::zeroed();
        sei.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        sei.fMask = SEE_MASK_NOASYNC;
        sei.lpVerb = verb.as_ptr();
        sei.lpFile = file.as_ptr();
        sei.lpParameters = params.as_ptr();
        if let Some(d) = &dir {
            sei.lpDirectory = d.as_ptr();
        }
        sei.nShow = show;
        if ShellExecuteExW(&mut sei) != 0 {
            Ok(())
        } else {
            Err(GetLastError())
        }
    }
}

fn launch_exe(exe: &Path) -> Result<(), u32> {
    let dir = exe.parent().map(|d| d.to_string_lossy().to_string());
    // HWiNFO 的清单要求管理员权限：已提权时直接启动，否则通过 UAC 请求
    let verb = if process_ops::is_elevated() { "open" } else { "runas" };
    shell_execute(verb, &exe.to_string_lossy(), "", dir.as_deref(), SW_SHOWMINNOACTIVE)
}

impl HwinfoManager {
    pub fn new(hub: Arc<SensorHub>, auto: bool) -> Arc<Self> {
        Arc::new(Self {
            hub,
            status: Mutex::new(HwinfoMgrStatus {
                auto,
                stage: "notInstalled".into(),
                ..Default::default()
            }),
            auto: AtomicBool::new(auto),
            declined: AtomicBool::new(false),
            force_launch: AtomicBool::new(false),
            launched_by_us: AtomicBool::new(false),
            kick: (Mutex::new(false), Condvar::new()),
            started: Instant::now(),
            last_launch: Mutex::new(None),
            last_restart: Mutex::new(None),
            rejected: Mutex::new(HashSet::new()),
            verified: Mutex::new(None),
        })
    }

    pub fn start(self: &Arc<Self>) {
        let m = self.clone();
        std::thread::Builder::new()
            .name("hwinfo-manager".into())
            .spawn(move || {
                unsafe {
                    use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
                    CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
                }
                m.run()
            })
            .ok();
    }

    pub fn status(&self) -> HwinfoMgrStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn set_auto(&self, auto: bool) {
        self.auto.store(auto, Ordering::SeqCst);
        self.status.lock().unwrap().auto = auto;
        self.wake();
    }

    fn wake(&self) {
        let (l, c) = &self.kick;
        *l.lock().unwrap() = true;
        c.notify_all();
    }

    /// 用户手动点击「启动 HWiNFO」
    pub fn request_launch(&self) {
        self.declined.store(false, Ordering::SeqCst);
        self.force_launch.store(true, Ordering::SeqCst);
        self.wake();
    }

    /// 以管理员权限启动前校验 REALiX 签名（结果按文件大小与修改时间缓存）
    fn verify(&self, exe: &Path) -> Result<(), String> {
        let meta = std::fs::metadata(exe).map_err(|e| format!("无法读取 {}：{}", exe.display(), e))?;
        let key = (exe.to_path_buf(), meta.len(), meta.modified().ok());
        if self.verified.lock().unwrap().as_ref() == Some(&key) {
            return Ok(());
        }
        let sig = process_ops::signature(&exe.to_string_lossy());
        let signer = sig.signer.clone().unwrap_or_default();
        if sig.status != "valid" || !signer.to_lowercase().contains("realix") {
            return Err(format!(
                "HWiNFO 数字签名校验未通过（{}，签名者：{}），出于安全考虑未启动",
                sig.status_text,
                if signer.is_empty() { "无" } else { &signer }
            ));
        }
        *self.verified.lock().unwrap() = Some(key);
        Ok(())
    }

    fn set(&self, f: impl FnOnce(&mut HwinfoMgrStatus)) {
        let mut g = self.status.lock().unwrap();
        f(&mut g);
        g.updated_at = now_ms();
    }

    /// 导入 HWiNFO 便携版（zip 或 HWiNFO64.exe），校验签名后复制到托管目录
    pub fn import(&self, src: &Path) -> Result<PathBuf, String> {
        if find_running().map_or(false, |(_, p)| p.map_or(false, |p| p.starts_with(managed_dir()))) {
            return Err("托管的 HWiNFO 正在运行，无法替换".into());
        }
        self.set(|s| {
            s.stage = "importing".into();
            s.message = Some(format!("正在导入 {}", src.display()));
        });
        let dest = managed_dir();
        std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        let is_zip = src
            .extension()
            .map_or(false, |e| e.to_string_lossy().eq_ignore_ascii_case("zip"));
        let tmp = dest.with_file_name("hwinfo_import_tmp");
        let _ = std::fs::remove_dir_all(&tmp);
        let exe = if is_zip {
            let script = format!(
                "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
                src.display().to_string().replace('\'', "''"),
                tmp.display().to_string().replace('\'', "''")
            );
            let ok = std::process::Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script])
                .creation_flags(CREATE_NO_WINDOW)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                return Err("解压失败，压缩包可能已损坏或尚未下载完成".into());
            }
            find_file_recursive(&tmp, "HWiNFO64.exe", 3).ok_or_else(|| {
                let _ = std::fs::remove_dir_all(&tmp);
                "压缩包中没有 HWiNFO64.exe，请下载 HWiNFO 的便携版（Portable）而不是安装版".to_string()
            })?
        } else {
            src.to_path_buf()
        };

        // 将以管理员权限运行，必须确认是 REALiX 签名的正版文件
        let sig = process_ops::signature(&exe.to_string_lossy());
        let signer = sig.signer.clone().unwrap_or_default();
        if sig.status != "valid" || !signer.to_lowercase().contains("realix") {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(format!(
                "数字签名校验未通过（{}，签名者：{}），已拒绝导入",
                sig.status_text,
                if signer.is_empty() { "无" } else { &signer }
            ));
        }

        let copy = |from: &Path| -> Result<(), String> {
            let name = from.file_name().unwrap_or_default();
            let to = dest.join(name);
            // 保留已有配置
            if to.exists() && name.to_string_lossy().to_lowercase().ends_with(".ini") {
                return Ok(());
            }
            std::fs::copy(from, &to).map(|_| ()).map_err(|e| format!("复制 {} 失败：{}", name.to_string_lossy(), e))
        };
        if is_zip {
            let dir = exe.parent().unwrap_or(&tmp).to_path_buf();
            for e in std::fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
                if e.path().is_file() {
                    copy(&e.path())?;
                }
            }
            let _ = std::fs::remove_dir_all(&tmp);
        } else {
            copy(&exe)?;
        }
        let target = dest.join("HWiNFO64.exe");
        configure_ini(&target);
        self.set(|s| {
            s.path = Some(target.display().to_string());
            s.managed = true;
            s.stage = "ready".into();
            s.message = Some("HWiNFO 已导入并完成配置".into());
        });
        Ok(target)
    }

    fn run(self: Arc<Self>) {
        loop {
            {
                let (l, c) = &self.kick;
                let mut g = l.lock().unwrap();
                if !*g {
                    g = c.wait_timeout(g, Duration::from_secs(5)).unwrap().0;
                }
                *g = false;
            }
            if self.hub.is_stopping() {
                break;
            }
            self.tick();
        }
    }

    fn tick(&self) {
        let auto = self.auto.load(Ordering::SeqCst);
        let force = self.force_launch.swap(false, Ordering::SeqCst);
        let mode = self.hub.mode();
        let running = find_running();
        let bundled_exe = if mode != SRC_LHM && (auto || force) { ensure_embedded() } else { None };
        let mut exe = running
            .as_ref()
            .and_then(|(_, p)| p.clone())
            .or(bundled_exe)
            .or_else(find_installed);

        // 自动导入用户下载的便携版
        if exe.is_none() && (auto || force) && mode != SRC_LHM {
            let cand = find_download(&self.rejected.lock().unwrap());
            if let Some(f) = cand {
                match self.import(&f) {
                    Ok(p) => exe = Some(p),
                    Err(e) => {
                        self.rejected.lock().unwrap().insert(f);
                        self.set(|s| {
                            s.stage = "error".into();
                            s.message = Some(e);
                        });
                        return;
                    }
                }
            }
        }

        let managed = exe.as_ref().map_or(false, |p| p.starts_with(managed_dir()));
        self.set(|s| {
            s.auto = auto;
            s.path = exe.as_ref().map(|p| p.display().to_string());
            s.managed = managed;
            s.bundled = exe.as_ref().map_or(false, |p| is_bundled_copy(p));
            s.running = running.is_some();
            s.launched_by_us = self.launched_by_us.load(Ordering::SeqCst);
        });

        if running.is_some() {
            let hw_state = self.hub.hwinfo_state();
            // 免费版共享内存 12 小时后失效：自动重启续期（仅限 FanGuard 启动或托管的实例）
            let ours = self.launched_by_us.load(Ordering::SeqCst) || managed;
            if hw_state == "inactive" && ours && (auto || force) {
                let recent = self
                    .last_restart
                    .lock()
                    .unwrap()
                    .map_or(false, |t| t.elapsed() < Duration::from_secs(20 * 60));
                if !recent {
                    if let Some(p) = &exe {
                        self.restart(running.as_ref().unwrap().0, p);
                        return;
                    }
                }
            }
            let msg = match hw_state.as_str() {
                "running" => None,
                "inactive" => Some("HWiNFO 共享内存已失效（免费版 12 小时限制）".to_string()),
                _ if !ours => Some("HWiNFO 正在运行，但未开启共享内存：请在 HWiNFO 设置中勾选「Shared Memory Support」".to_string()),
                _ => Some("HWiNFO 正在启动，等待共享内存就绪…".to_string()),
            };
            self.set(|s| {
                s.stage = "running".into();
                s.message = msg;
            });
            return;
        }

        if mode == SRC_LHM || (!auto && !force) {
            self.set(|s| {
                s.stage = if !auto { "disabled".into() } else if exe.is_some() { "ready".into() } else { "notInstalled".into() };
                s.message = None;
            });
            return;
        }

        let Some(exe) = exe else {
            self.set(|s| {
                s.stage = "notInstalled".into();
                s.message = Some("未找到 HWiNFO。下载便携版到「下载」文件夹或 FanGuard 所在目录后会自动导入，无需解压".into());
            });
            return;
        };

        // 自动模式下仅当 LibreHardwareMonitor 读不到关键数据时才启动，避免无谓的 UAC 提示
        let need = force || mode == SRC_HWINFO || {
            let warm = self.started.elapsed() > Duration::from_secs(20);
            let lhm = self.hub.lhm_summary();
            warm && (lhm.cpu_temp.is_none() || lhm.fans.is_empty())
        };
        if !need {
            self.set(|s| {
                s.stage = "ready".into();
                s.message = Some("LibreHardwareMonitor 已能读取 CPU 温度与风扇，暂不需要启动 HWiNFO".into());
            });
            return;
        }
        if self.declined.load(Ordering::SeqCst) && !force {
            self.set(|s| {
                s.stage = "declined".into();
                s.message = Some("已取消管理员授权，HWiNFO 未启动。可点击「启动 HWiNFO」重试".into());
            });
            return;
        }
        let recent = self
            .last_launch
            .lock()
            .unwrap()
            .map_or(false, |t| t.elapsed() < Duration::from_secs(60));
        if recent && !force {
            return;
        }

        if let Err(e) = self.verify(&exe) {
            *self.last_launch.lock().unwrap() = Some(Instant::now());
            self.set(|s| {
                s.stage = "error".into();
                s.message = Some(e);
            });
            return;
        }
        let configured = configure_ini(&exe);
        self.set(|s| {
            s.stage = "launching".into();
            s.configured = Some(configured);
            s.message = Some(if process_ops::is_elevated() {
                "正在启动 HWiNFO…".into()
            } else {
                "正在启动 HWiNFO…（HWiNFO 需要管理员权限，请在 UAC 提示中点击「是」）".into()
            });
        });
        *self.last_launch.lock().unwrap() = Some(Instant::now());
        match launch_exe(&exe) {
            Ok(()) => {
                self.launched_by_us.store(true, Ordering::SeqCst);
                if !configured {
                    self.set(|s| {
                        s.message = Some("HWiNFO 已启动，但无法写入其配置文件（安装目录需要管理员权限）。如未自动连接，请在 HWiNFO 设置中勾选「Shared Memory Support」".into());
                    });
                }
            }
            Err(ERROR_CANCELLED) => {
                self.declined.store(true, Ordering::SeqCst);
                self.set(|s| {
                    s.stage = "declined".into();
                    s.message = Some("已取消管理员授权，HWiNFO 未启动。可点击「启动 HWiNFO」重试".into());
                });
            }
            Err(code) => self.set(|s| {
                s.stage = "error".into();
                s.message = Some(format!("启动 HWiNFO 失败（错误码 {}）", code));
            }),
        }
    }

    fn restart(&self, pid: u32, exe: &Path) {
        *self.last_restart.lock().unwrap() = Some(Instant::now());
        if let Err(e) = self.verify(exe) {
            self.set(|s| s.message = Some(e));
            return;
        }
        configure_ini(exe);
        self.set(|s| s.message = Some("HWiNFO 共享内存已到期，正在自动重启 HWiNFO…".into()));
        let r = if process_ops::is_elevated() {
            let _ = process_ops::terminate(pid);
            std::thread::sleep(Duration::from_secs(2));
            launch_exe(exe)
        } else {
            // 未提权时无法结束管理员进程：用一次 UAC 授权完成「结束 + 重新启动」
            let script = format!(
                "Stop-Process -Id {} -Force -ErrorAction SilentlyContinue; Start-Sleep -Seconds 2; Start-Process -FilePath '{}' -WindowStyle Minimized",
                pid,
                exe.display().to_string().replace('\'', "''")
            );
            shell_execute(
                "runas",
                "powershell.exe",
                &format!("-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -Command \"{}\"", script),
                None,
                SW_HIDE,
            )
        };
        match r {
            Ok(()) => self.launched_by_us.store(true, Ordering::SeqCst),
            Err(ERROR_CANCELLED) => self.set(|s| {
                s.message = Some("已取消重启 HWiNFO 的授权，共享内存仍处于失效状态".into());
            }),
            Err(code) => self.set(|s| s.message = Some(format!("重启 HWiNFO 失败（错误码 {}）", code))),
        }
    }

    /// 退出 FanGuard 时关闭由它启动的 HWiNFO（未提权时无权结束管理员进程，保持运行）
    pub fn on_exit(&self) {
        if self.launched_by_us.load(Ordering::SeqCst) && process_ops::is_elevated() {
            if let Some((pid, _)) = find_running() {
                let _ = process_ops::terminate(pid);
            }
        }
    }
}

pub fn open_download_page() -> Result<(), String> {
    shell_execute("open", "https://www.hwinfo.com/download/", "", None, 1).map_err(|c| format!("无法打开浏览器（错误码 {}）", c))
}
