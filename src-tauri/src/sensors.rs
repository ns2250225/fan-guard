//! 硬件传感器：LibreHardwareMonitor Sidecar + HWiNFO 共享内存 + NVML 回退
//!
//! Sidecar 进程常驻后台，按设置的间隔输出 JSON；本模块负责启动、守护、解析，
//! 并汇总出 CPU / GPU 温度、功耗、频率以及风扇转速。
//! 硬件不支持或数据不可用时字段为 None，前端显示「暂不支持读取」，绝不虚构数据。

use crate::util::{now_ms, CREATE_NO_WINDOW};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorNode {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub sensor_type: String,
    pub value: Option<f64>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    /// 单位（HWiNFO 提供；LibreHardwareMonitor 由类型推断）
    #[serde(default)]
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HwNode {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub hw_type: String,
    pub parent: Option<String>,
    #[serde(default)]
    pub parent_type: Option<String>,
    #[serde(default)]
    pub parent_name: Option<String>,
    pub sensors: Vec<SensorNode>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SidecarStatus {
    /// 未找到 Sidecar 可执行文件
    NotFound,
    Starting,
    Running,
    /// 已退出（会自动重启）
    Exited,
    /// 监控已暂停
    Paused,
    /// 数据来源设置为仅 HWiNFO，未启动
    Disabled,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SensorStatus {
    pub status: SidecarStatus,
    pub admin: bool,
    pub message: Option<String>,
    pub lhm_version: Option<String>,
    pub updated_at: i64,
    pub nvml: bool,
    /// 实际使用的数据来源
    pub source: String,
    /// 数据来源设置：auto / lhm / hwinfo
    pub mode: String,
    /// HWiNFO 状态：disabled / notFound / inactive / invalid / stale / running
    pub hwinfo: String,
    pub hwinfo_sensors: usize,
    /// 笔记本 EC（Uniwill/同方模具）：available / needAdmin / none
    pub ec: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GpuSensor {
    pub name: String,
    pub temp: Option<f64>,
    pub hotspot: Option<f64>,
    pub power: Option<f64>,
    pub clock: Option<f64>,
    pub mem_clock: Option<f64>,
    pub load: Option<f64>,
    pub mem_used: Option<u64>,
    pub mem_total: Option<u64>,
    pub fan_rpm: Option<f64>,
    pub fan_percent: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FanReading {
    pub name: String,
    pub hardware: String,
    /// cpu / gpu / case
    pub kind: String,
    pub rpm: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TempReading {
    pub name: String,
    pub hardware: String,
    pub hw_type: String,
    pub value: f64,
    pub max: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SensorSummary {
    pub cpu_temp: Option<f64>,
    pub cpu_power: Option<f64>,
    pub cpu_clock: Option<f64>,
    pub cpu_core_clocks: Vec<f64>,
    pub gpus: Vec<GpuSensor>,
    pub fans: Vec<FanReading>,
    pub temps: Vec<TempReading>,
}

struct Inner {
    status: SidecarStatus,
    admin: bool,
    message: Option<String>,
    lhm_version: Option<String>,
    hardware: Vec<HwNode>,
    updated_at: i64,
    nvml_gpus: Vec<GpuSensor>,
    hwinfo_state: String,
    hwinfo_nodes: Vec<HwNode>,
    ec: String,
}

pub const SRC_AUTO: u8 = 0;
pub const SRC_LHM: u8 = 1;
pub const SRC_HWINFO: u8 = 2;

pub struct SensorHub {
    inner: Mutex<Inner>,
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Option<Child>>,
    interval_ms: AtomicU64,
    stopping: AtomicBool,
    paused: AtomicBool,
    nvml_enabled: AtomicBool,
    source: AtomicU8,
}

/// 构建时嵌入主程序的传感器 Sidecar（单文件发布）
#[cfg(embedded_sidecar)]
static EMBEDDED_SIDECAR: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/binaries/fanguard-sensors-x86_64-pc-windows-msvc.exe"
));

/// 将内嵌的 Sidecar 释放到 %LOCALAPPDATA%\FanGuard\sensors，文件名带内容哈希，
/// 版本未变化时直接复用，不会每次启动都重新写入
#[cfg(embedded_sidecar)]
fn extract_embedded() -> Option<PathBuf> {
    let base = std::env::var("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let dir = base.join("FanGuard").join("sensors");
    std::fs::create_dir_all(&dir).ok()?;
    let name = format!("fanguard-sensors-{}.exe", env!("FANGUARD_SIDECAR_HASH"));
    let target = dir.join(&name);
    let ok = std::fs::metadata(&target)
        .map(|m| m.len() == EMBEDDED_SIDECAR.len() as u64)
        .unwrap_or(false);
    if !ok {
        let tmp = dir.join(format!("{}.{}.tmp", name, std::process::id()));
        std::fs::write(&tmp, EMBEDDED_SIDECAR).ok()?;
        if std::fs::rename(&tmp, &target).is_err() {
            let _ = std::fs::remove_file(&tmp);
            // 可能另一个实例刚刚写入完成
            if !target.exists() {
                return None;
            }
        }
    }
    // 清理旧版本
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n != name && n.starts_with("fanguard-sensors-") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    Some(target)
}

fn sidecar_path() -> Option<PathBuf> {
    #[cfg(embedded_sidecar)]
    if let Some(p) = extract_embedded() {
        return Some(p);
    }
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let candidates = [
        dir.join("fanguard-sensors.exe"),
        dir.join("fanguard-sensors-x86_64-pc-windows-msvc.exe"),
        dir.join("binaries").join("fanguard-sensors-x86_64-pc-windows-msvc.exe"),
        dir.join("..").join("..").join("binaries").join("fanguard-sensors-x86_64-pc-windows-msvc.exe"),
    ];
    candidates.into_iter().find(|p| p.exists())
}

impl SensorHub {
    pub fn new(interval_ms: u64) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                status: SidecarStatus::Starting,
                admin: false,
                message: None,
                lhm_version: None,
                hardware: vec![],
                updated_at: 0,
                nvml_gpus: vec![],
                hwinfo_state: "notFound".into(),
                hwinfo_nodes: vec![],
                ec: "none".into(),
            }),
            stdin: Mutex::new(None),
            child: Mutex::new(None),
            interval_ms: AtomicU64::new(interval_ms),
            stopping: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            nvml_enabled: AtomicBool::new(false),
            source: AtomicU8::new(SRC_AUTO),
        })
    }

    fn source(&self) -> u8 {
        self.source.load(Ordering::SeqCst)
    }

    pub fn mode(&self) -> u8 {
        self.source()
    }

    pub fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::SeqCst)
    }

    pub fn hwinfo_state(&self) -> String {
        self.inner.lock().unwrap().hwinfo_state.clone()
    }

    /// 仅 LibreHardwareMonitor 的汇总，用于判断是否需要启动 HWiNFO
    pub fn lhm_summary(&self) -> SensorSummary {
        let g = self.inner.lock().unwrap();
        if now_ms() - g.updated_at < 15_000 {
            summarize(&g.hardware)
        } else {
            SensorSummary::default()
        }
    }

    /// 切换数据来源：auto（HWiNFO 优先，缺失项由 LibreHardwareMonitor 补充）/ lhm / hwinfo
    pub fn set_source(&self, mode: &str) {
        let v = match mode {
            "lhm" => SRC_LHM,
            "hwinfo" => SRC_HWINFO,
            _ => SRC_AUTO,
        };
        let old = self.source.swap(v, Ordering::SeqCst);
        if v == SRC_HWINFO && old != SRC_HWINFO {
            // 仅使用 HWiNFO 时停止 LibreHardwareMonitor，节省资源；GPU 允许 NVML 兜底
            self.send("quit");
            self.nvml_enabled.store(true, Ordering::SeqCst);
        }
        if v == SRC_LHM {
            let mut g = self.inner.lock().unwrap();
            g.hwinfo_nodes.clear();
            g.hwinfo_state = "disabled".into();
        }
    }

    /// 启动守护线程：负责启动 Sidecar、异常退出后自动重启
    pub fn start(self: &Arc<Self>) {
        let hub = self.clone();
        std::thread::Builder::new()
            .name("sensor-supervisor".into())
            .spawn(move || hub.supervise())
            .ok();
        let hub = self.clone();
        std::thread::Builder::new()
            .name("nvml".into())
            .spawn(move || hub.nvml_loop())
            .ok();
        let hub = self.clone();
        std::thread::Builder::new()
            .name("hwinfo".into())
            .spawn(move || hub.hwinfo_loop())
            .ok();
    }

    /// 轮询 HWiNFO 共享内存（只读，开销很小；HWiNFO 未运行时立即返回）
    fn hwinfo_loop(self: Arc<Self>) {
        while !self.stopping.load(Ordering::SeqCst) {
            let interval = self.interval_ms.load(Ordering::SeqCst).max(500);
            if self.source() != SRC_LHM && !self.paused.load(Ordering::SeqCst) {
                let r = crate::hwinfo::read();
                let mut g = self.inner.lock().unwrap();
                match r {
                    Ok(d) => {
                        let now_s = now_ms() / 1000;
                        // HWiNFO 被挂起或停止轮询超过 30 秒：数据视为过期，不再使用
                        if d.poll_time > 0 && now_s - d.poll_time > 30 {
                            g.hwinfo_state = "stale".into();
                            g.hwinfo_nodes.clear();
                        } else {
                            g.hwinfo_state = "running".into();
                            g.hwinfo_nodes = d.nodes;
                        }
                    }
                    Err(code) => {
                        g.hwinfo_state = code.into();
                        g.hwinfo_nodes.clear();
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(interval));
        }
    }

    fn set_status(&self, status: SidecarStatus, message: Option<String>) {
        let mut g = self.inner.lock().unwrap();
        g.status = status;
        if message.is_some() {
            g.message = message;
        }
    }

    fn supervise(self: Arc<Self>) {
        let mut failures = 0u32;
        while !self.stopping.load(Ordering::SeqCst) {
            if self.source() == SRC_HWINFO {
                {
                    let mut g = self.inner.lock().unwrap();
                    g.status = SidecarStatus::Disabled;
                    g.hardware.clear();
                }
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
            let path = match sidecar_path() {
                Some(p) => p,
                None => {
                    self.set_status(
                        SidecarStatus::NotFound,
                        Some("未找到硬件传感器服务 fanguard-sensors.exe，温度与风扇数据不可用".into()),
                    );
                    self.nvml_enabled.store(true, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_secs(30));
                    continue;
                }
            };
            self.set_status(SidecarStatus::Starting, None);
            let started = std::time::Instant::now();
            let spawn = Command::new(&path)
                .args([
                    "--interval",
                    &self.interval_ms.load(Ordering::SeqCst).to_string(),
                    "--parent",
                    &std::process::id().to_string(),
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .spawn();
            let mut child = match spawn {
                Ok(c) => c,
                Err(e) => {
                    self.set_status(SidecarStatus::Exited, Some(format!("无法启动硬件传感器服务：{}", e)));
                    self.nvml_enabled.store(true, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_secs(30));
                    continue;
                }
            };
            let stdout = child.stdout.take();
            *self.stdin.lock().unwrap() = child.stdin.take();
            *self.child.lock().unwrap() = Some(child);
            if self.paused.load(Ordering::SeqCst) {
                self.send("pause");
            }

            if let Some(out) = stdout {
                let reader = BufReader::new(out);
                for line in reader.lines() {
                    let line = match line {
                        Ok(l) => l,
                        Err(_) => break,
                    };
                    self.handle_line(&line);
                }
            }

            // 读取结束 —— 进程已退出
            if let Some(mut c) = self.child.lock().unwrap().take() {
                let _ = c.kill();
                let _ = c.wait();
            }
            *self.stdin.lock().unwrap() = None;
            if self.stopping.load(Ordering::SeqCst) {
                break;
            }
            if self.source() == SRC_HWINFO {
                // 用户切换为仅 HWiNFO 导致的正常退出
                failures = 0;
                continue;
            }
            self.nvml_enabled.store(true, Ordering::SeqCst);
            {
                let mut g = self.inner.lock().unwrap();
                g.status = SidecarStatus::Exited;
                g.hardware.clear();
                g.message = Some("硬件传感器服务意外退出，正在尝试重新启动".into());
            }
            if started.elapsed() > Duration::from_secs(60) {
                failures = 0;
            }
            failures += 1;
            let wait = if failures <= 3 { 3 } else { 60 };
            std::thread::sleep(Duration::from_secs(wait));
        }
    }

    fn handle_line(&self, line: &str) {
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return,
        };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("hello") => {
                let mut g = self.inner.lock().unwrap();
                g.admin = v.get("admin").and_then(|a| a.as_bool()).unwrap_or(false);
                g.lhm_version = v.get("lhm").and_then(|a| a.as_str()).map(|s| s.to_string());
                g.ec = v.get("ec").and_then(|a| a.as_str()).unwrap_or("none").to_string();
                g.status = SidecarStatus::Running;
                g.message = if g.admin {
                    None
                } else if g.ec == "needAdmin" {
                    Some("检测到同方 / Uniwill 模具笔记本的嵌入式控制器：风扇转速需要以管理员身份运行 FanGuard 才能读取".into())
                } else {
                    Some("当前未以管理员身份运行，部分 CPU 温度、功耗与主板风扇传感器可能无法读取".into())
                };
            }
            Some("data") => {
                if let Some(hw) = v.get("hardware") {
                    if let Ok(list) = serde_json::from_value::<Vec<HwNode>>(hw.clone()) {
                        let has_gpu = list.iter().any(|h| h.hw_type.starts_with("Gpu"));
                        let mut g = self.inner.lock().unwrap();
                        g.hardware = list;
                        g.updated_at = now_ms();
                        if !self.paused.load(Ordering::SeqCst) {
                            g.status = SidecarStatus::Running;
                        }
                        drop(g);
                        // LHM 已提供 GPU 数据时无需 NVML
                        self.nvml_enabled.store(!has_gpu, Ordering::SeqCst);
                    }
                }
            }
            Some("error") => {
                let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("").to_string();
                self.inner.lock().unwrap().message = Some(msg);
            }
            _ => {}
        }
    }

    fn send(&self, cmd: &str) {
        if let Some(stdin) = self.stdin.lock().unwrap().as_mut() {
            let _ = writeln!(stdin, "{}", cmd);
            let _ = stdin.flush();
        }
    }

    pub fn set_interval(&self, ms: u64) {
        self.interval_ms.store(ms, Ordering::SeqCst);
        self.send(&format!("interval {}", ms));
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::SeqCst);
        self.send(if paused { "pause" } else { "resume" });
        let mut g = self.inner.lock().unwrap();
        if paused && g.status == SidecarStatus::Running {
            g.status = SidecarStatus::Paused;
        } else if !paused && g.status == SidecarStatus::Paused {
            g.status = SidecarStatus::Running;
        }
    }

    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.send("quit");
        if let Some(mut c) = self.child.lock().unwrap().take() {
            std::thread::sleep(Duration::from_millis(200));
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    pub fn status(&self) -> SensorStatus {
        let g = self.inner.lock().unwrap();
        let mode = self.source();
        let nvml = !g.nvml_gpus.is_empty();
        let lhm_ok = mode != SRC_HWINFO && !g.hardware.is_empty() && now_ms() - g.updated_at < 15_000;
        let hw_ok = mode != SRC_LHM && !g.hwinfo_nodes.is_empty();
        let source = match (hw_ok, lhm_ok) {
            (true, true) => "HWiNFO + LibreHardwareMonitor",
            (true, false) => "HWiNFO",
            (false, true) => "LibreHardwareMonitor",
            _ if nvml => "NVML",
            _ => "none",
        };
        let message = if hw_ok {
            None
        } else if mode == SRC_HWINFO {
            Some(hwinfo_hint(&g.hwinfo_state).to_string())
        } else {
            g.message.clone()
        };
        SensorStatus {
            status: if mode == SRC_HWINFO { SidecarStatus::Disabled } else { g.status.clone() },
            admin: g.admin,
            message,
            lhm_version: g.lhm_version.clone(),
            updated_at: g.updated_at,
            nvml,
            source: source.into(),
            mode: match mode {
                SRC_LHM => "lhm",
                SRC_HWINFO => "hwinfo",
                _ => "auto",
            }
            .into(),
            hwinfo: if mode == SRC_LHM { "disabled".into() } else { g.hwinfo_state.clone() },
            hwinfo_sensors: g.hwinfo_nodes.iter().map(|n| n.sensors.len()).sum(),
            ec: g.ec.clone(),
        }
    }

    pub fn raw(&self) -> Vec<HwNode> {
        let g = self.inner.lock().unwrap();
        let mut v = g.hardware.clone();
        v.extend(g.hwinfo_nodes.iter().cloned());
        v
    }

    pub fn summary(&self) -> SensorSummary {
        let g = self.inner.lock().unwrap();
        let mode = self.source();
        // 数据超过 15 秒未更新视为失效
        let fresh = now_ms() - g.updated_at < 15_000;
        let lhm = if mode != SRC_HWINFO && fresh {
            summarize(&g.hardware)
        } else {
            SensorSummary::default()
        };
        let hw_ok = mode != SRC_LHM && !g.hwinfo_nodes.is_empty();
        let mut s = if hw_ok {
            // HWiNFO 优先（通常传感器更全），缺失项由 LibreHardwareMonitor 补充
            merge(crate::hwinfo::summarize(&g.hwinfo_nodes), lhm)
        } else {
            lhm
        };
        if s.gpus.is_empty() && !g.nvml_gpus.is_empty() {
            s.gpus = g.nvml_gpus.clone();
        }
        s
    }

    fn nvml_loop(self: Arc<Self>) {
        let mut nvml: Option<Nvml> = None;
        let mut tried = false;
        while !self.stopping.load(Ordering::SeqCst) {
            let interval = self.interval_ms.load(Ordering::SeqCst).max(500);
            if self.nvml_enabled.load(Ordering::SeqCst) && !self.paused.load(Ordering::SeqCst) {
                if nvml.is_none() && !tried {
                    tried = true;
                    nvml = Nvml::load();
                }
                if let Some(n) = &nvml {
                    let gpus = n.read();
                    self.inner.lock().unwrap().nvml_gpus = gpus;
                }
            } else if !self.nvml_enabled.load(Ordering::SeqCst) {
                let mut g = self.inner.lock().unwrap();
                if !g.nvml_gpus.is_empty() {
                    g.nvml_gpus.clear();
                }
            }
            std::thread::sleep(Duration::from_millis(interval));
        }
    }
}

fn hwinfo_hint(state: &str) -> &'static str {
    match state {
        "inactive" => "HWiNFO 共享内存已关闭：请在 HWiNFO 设置中重新勾选「Shared Memory Support」（免费版每 12 小时需重新启用）",
        "stale" => "HWiNFO 数据已超过 30 秒未更新，请确认 HWiNFO 传感器窗口正在运行",
        "invalid" => "HWiNFO 共享内存格式无法识别，请升级到最新版 HWiNFO",
        _ => "未检测到 HWiNFO 共享内存，FanGuard 会自动查找、配置并启动 HWiNFO",
    }
}

fn same_gpu(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.to_lowercase()
            .replace("(r)", "")
            .replace("(tm)", "")
            .replace("nvidia", "")
            .replace("amd", "")
            .replace("intel", "")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    };
    let (a, b) = (norm(a), norm(b));
    !a.is_empty() && !b.is_empty() && (a.contains(&b) || b.contains(&a))
}

/// 以 primary 为准，用 fallback 补齐缺失的数据
fn merge(mut p: SensorSummary, f: SensorSummary) -> SensorSummary {
    if p.cpu_temp.is_none() {
        p.cpu_temp = f.cpu_temp;
    }
    if p.cpu_power.is_none() {
        p.cpu_power = f.cpu_power;
    }
    if p.cpu_clock.is_none() {
        p.cpu_clock = f.cpu_clock;
    }
    if p.cpu_core_clocks.is_empty() {
        p.cpu_core_clocks = f.cpu_core_clocks;
    }
    if p.gpus.is_empty() {
        p.gpus = f.gpus;
    } else {
        for g in p.gpus.iter_mut() {
            if let Some(o) = f.gpus.iter().find(|o| same_gpu(&g.name, &o.name)) {
                g.temp = g.temp.or(o.temp);
                g.hotspot = g.hotspot.or(o.hotspot);
                g.power = g.power.or(o.power);
                g.clock = g.clock.or(o.clock);
                g.mem_clock = g.mem_clock.or(o.mem_clock);
                g.load = g.load.or(o.load);
                g.mem_used = g.mem_used.or(o.mem_used);
                g.mem_total = g.mem_total.or(o.mem_total);
                g.fan_rpm = g.fan_rpm.or(o.fan_rpm);
                g.fan_percent = g.fan_percent.or(o.fan_percent);
            }
        }
    }
    if p.fans.is_empty() {
        p.fans = f.fans;
    }
    if p.temps.is_empty() {
        p.temps = f.temps;
    } else if !p.temps.iter().any(|t| t.hw_type == "Storage") {
        p.temps.extend(f.temps.into_iter().filter(|t| t.hw_type == "Storage"));
    }
    p
}

// ---------------------------------------------------------------------------
// 传感器汇总
// ---------------------------------------------------------------------------

fn find<'a>(hw: &'a HwNode, stype: &str, names: &[&str]) -> Option<f64> {
    for n in names {
        if let Some(s) = hw
            .sensors
            .iter()
            .find(|s| s.sensor_type == stype && s.name.eq_ignore_ascii_case(n) && s.value.is_some())
        {
            return s.value;
        }
    }
    None
}

fn first(hw: &HwNode, stype: &str) -> Option<f64> {
    hw.sensors
        .iter()
        .find(|s| s.sensor_type == stype && s.value.is_some())
        .and_then(|s| s.value)
}

fn valid_temp(v: Option<f64>) -> Option<f64> {
    v.filter(|t| *t > 1.0 && *t < 150.0)
}

pub fn summarize(list: &[HwNode]) -> SensorSummary {
    let mut s = SensorSummary::default();

    for hw in list {
        let t = hw.hw_type.as_str();
        if t == "Cpu" {
            if s.cpu_temp.is_none() {
                s.cpu_temp = valid_temp(find(
                    hw,
                    "Temperature",
                    &[
                        "CPU Package",
                        "Core (Tctl/Tdie)",
                        "Core (Tdie)",
                        "Core (Tctl)",
                        "Tctl",
                        "Core Max",
                        "Core Average",
                        "CPU Cores",
                    ],
                ))
                .or_else(|| {
                    valid_temp(
                        hw.sensors
                            .iter()
                            .filter(|x| x.sensor_type == "Temperature")
                            .filter_map(|x| x.value)
                            .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v)))),
                    )
                });
            }
            if s.cpu_power.is_none() {
                s.cpu_power = find(hw, "Power", &["CPU Package", "Package", "CPU Cores"]).or_else(|| first(hw, "Power"));
            }
            let clocks: Vec<f64> = hw
                .sensors
                .iter()
                .filter(|x| x.sensor_type == "Clock" && x.name.starts_with("Core #") && !x.name.contains("Effective"))
                .filter_map(|x| x.value)
                .filter(|v| *v > 0.0)
                .collect();
            if !clocks.is_empty() && s.cpu_clock.is_none() {
                s.cpu_clock = Some(clocks.iter().sum::<f64>() / clocks.len() as f64);
                s.cpu_core_clocks = clocks;
            } else if s.cpu_clock.is_none() {
                s.cpu_clock = find(hw, "Clock", &["Cores (Average)", "Core (Average)"]);
            }
        } else if t.starts_with("Gpu") {
            let mem_used = find(hw, "SmallData", &["GPU Memory Used", "D3D Dedicated Memory Used"]);
            let mem_total = find(hw, "SmallData", &["GPU Memory Total"]);
            s.gpus.push(GpuSensor {
                name: hw.name.clone(),
                temp: valid_temp(find(hw, "Temperature", &["GPU Core", "GPU Temperature"]).or_else(|| first(hw, "Temperature"))),
                hotspot: valid_temp(find(hw, "Temperature", &["GPU Hot Spot", "GPU Hotspot"])),
                power: find(hw, "Power", &["GPU Package", "GPU Power", "GPU Board Power", "GPU Core"]).or_else(|| first(hw, "Power")),
                clock: find(hw, "Clock", &["GPU Core"]),
                mem_clock: find(hw, "Clock", &["GPU Memory"]),
                load: find(hw, "Load", &["GPU Core", "D3D 3D"]),
                mem_used: mem_used.map(|mb| (mb * 1024.0 * 1024.0) as u64),
                mem_total: mem_total.map(|mb| (mb * 1024.0 * 1024.0) as u64),
                fan_rpm: hw
                    .sensors
                    .iter()
                    .filter(|x| x.sensor_type == "Fan")
                    .filter_map(|x| x.value)
                    .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v)))),
                fan_percent: find(hw, "Control", &["GPU Fan"]).or_else(|| first(hw, "Control")),
            });
        }

        // 风扇
        let parent_name = hw.parent_name.clone();
        let hw_label = match &parent_name {
            Some(p) => format!("{} / {}", p, hw.name),
            None => hw.name.clone(),
        };
        for sensor in hw.sensors.iter().filter(|x| x.sensor_type == "Fan") {
            let v = match sensor.value {
                Some(v) => v,
                None => continue,
            };
            // 未接风扇的接口通常始终为 0，过滤掉
            if v <= 0.0 && sensor.max.unwrap_or(0.0) <= 0.0 {
                continue;
            }
            let lname = sensor.name.to_lowercase();
            let kind = if t.starts_with("Gpu") || lname.contains("gpu") {
                "gpu"
            } else if lname.contains("cpu") || lname.contains("pump") || lname.contains("aio") || (lname == "fan #1" && t == "SuperIO") {
                "cpu"
            } else {
                "case"
            };
            s.fans.push(FanReading {
                name: sensor.name.clone(),
                hardware: hw_label.clone(),
                kind: kind.into(),
                rpm: v,
            });
        }

        // 温度
        for sensor in hw.sensors.iter().filter(|x| x.sensor_type == "Temperature") {
            if let Some(v) = valid_temp(sensor.value) {
                s.temps.push(TempReading {
                    name: sensor.name.clone(),
                    hardware: hw_label.clone(),
                    hw_type: hw.hw_type.clone(),
                    value: v,
                    max: sensor.max,
                });
            }
        }
    }

    // 笔记本：CPU 温度可能只在嵌入式控制器/主板上提供
    if s.cpu_temp.is_none() {
        s.cpu_temp = s
            .temps
            .iter()
            .find(|t| t.hw_type != "Cpu" && !t.hw_type.starts_with("Gpu") && t.name.to_lowercase().contains("cpu"))
            .map(|t| t.value);
    }
    s
}

impl SensorSummary {
    /// 用于诊断与历史记录的代表性风扇转速：优先 CPU 风扇，否则取全部风扇中的最大值
    pub fn main_fan_rpm(&self) -> Option<f64> {
        let cpu = self
            .fans
            .iter()
            .filter(|f| f.kind == "cpu")
            .map(|f| f.rpm)
            .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))));
        cpu.or_else(|| {
            self.fans
                .iter()
                .map(|f| f.rpm)
                .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))))
        })
    }
}

// ---------------------------------------------------------------------------
// NVML（NVIDIA 显卡回退方案，无需管理员权限）
// ---------------------------------------------------------------------------

type NvmlDevice = *mut std::ffi::c_void;

#[repr(C)]
#[derive(Default)]
struct NvmlUtilization {
    gpu: u32,
    memory: u32,
}

#[repr(C)]
#[derive(Default)]
struct NvmlMemory {
    total: u64,
    free: u64,
    used: u64,
}

struct Nvml {
    _lib: libloading::Library,
    get_count: unsafe extern "C" fn(*mut u32) -> i32,
    get_handle: unsafe extern "C" fn(u32, *mut NvmlDevice) -> i32,
    get_name: unsafe extern "C" fn(NvmlDevice, *mut u8, u32) -> i32,
    get_temp: unsafe extern "C" fn(NvmlDevice, u32, *mut u32) -> i32,
    get_power: unsafe extern "C" fn(NvmlDevice, *mut u32) -> i32,
    get_clock: unsafe extern "C" fn(NvmlDevice, u32, *mut u32) -> i32,
    get_util: unsafe extern "C" fn(NvmlDevice, *mut NvmlUtilization) -> i32,
    get_mem: unsafe extern "C" fn(NvmlDevice, *mut NvmlMemory) -> i32,
    get_fan: unsafe extern "C" fn(NvmlDevice, *mut u32) -> i32,
}

unsafe impl Send for Nvml {}

impl Nvml {
    fn load() -> Option<Self> {
        let sysroot = crate::util::system_root();
        let paths = [
            format!("{}\\System32\\nvml.dll", sysroot),
            "C:\\Program Files\\NVIDIA Corporation\\NVSMI\\nvml.dll".to_string(),
            "nvml.dll".to_string(),
        ];
        for p in paths {
            unsafe {
                let lib = match libloading::Library::new(&p) {
                    Ok(l) => l,
                    Err(_) => continue,
                };
                let init: libloading::Symbol<unsafe extern "C" fn() -> i32> = match lib.get(b"nvmlInit_v2\0") {
                    Ok(f) => f,
                    Err(_) => continue,
                };
                if init() != 0 {
                    continue;
                }
                macro_rules! sym {
                    ($name:expr) => {
                        match lib.get($name) {
                            Ok(f) => *f,
                            Err(_) => continue,
                        }
                    };
                }
                let n = Nvml {
                    get_count: sym!(b"nvmlDeviceGetCount_v2\0"),
                    get_handle: sym!(b"nvmlDeviceGetHandleByIndex_v2\0"),
                    get_name: sym!(b"nvmlDeviceGetName\0"),
                    get_temp: sym!(b"nvmlDeviceGetTemperature\0"),
                    get_power: sym!(b"nvmlDeviceGetPowerUsage\0"),
                    get_clock: sym!(b"nvmlDeviceGetClockInfo\0"),
                    get_util: sym!(b"nvmlDeviceGetUtilizationRates\0"),
                    get_mem: sym!(b"nvmlDeviceGetMemoryInfo\0"),
                    get_fan: sym!(b"nvmlDeviceGetFanSpeed\0"),
                    _lib: lib,
                };
                return Some(n);
            }
        }
        None
    }

    fn read(&self) -> Vec<GpuSensor> {
        let mut out = vec![];
        unsafe {
            let mut count = 0u32;
            if (self.get_count)(&mut count) != 0 {
                return out;
            }
            for i in 0..count {
                let mut dev: NvmlDevice = std::ptr::null_mut();
                if (self.get_handle)(i, &mut dev) != 0 {
                    continue;
                }
                let mut name = [0u8; 96];
                let name = if (self.get_name)(dev, name.as_mut_ptr(), name.len() as u32) == 0 {
                    let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
                    String::from_utf8_lossy(&name[..end]).to_string()
                } else {
                    "NVIDIA GPU".into()
                };
                let mut g = GpuSensor {
                    name,
                    ..Default::default()
                };
                let mut v = 0u32;
                if (self.get_temp)(dev, 0, &mut v) == 0 {
                    g.temp = Some(v as f64);
                }
                if (self.get_power)(dev, &mut v) == 0 {
                    g.power = Some(v as f64 / 1000.0);
                }
                if (self.get_clock)(dev, 0, &mut v) == 0 {
                    g.clock = Some(v as f64);
                }
                if (self.get_clock)(dev, 2, &mut v) == 0 {
                    g.mem_clock = Some(v as f64);
                }
                let mut u = NvmlUtilization::default();
                if (self.get_util)(dev, &mut u) == 0 {
                    g.load = Some(u.gpu as f64);
                }
                let mut m = NvmlMemory::default();
                if (self.get_mem)(dev, &mut m) == 0 {
                    g.mem_used = Some(m.used);
                    g.mem_total = Some(m.total);
                }
                // NVML 只提供风扇占空比百分比，不是 RPM，不能当作转速显示
                if (self.get_fan)(dev, &mut v) == 0 {
                    g.fan_percent = Some(v as f64);
                }
                out.push(g);
            }
        }
        out
    }
}
