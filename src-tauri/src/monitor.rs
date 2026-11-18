//! 应用状态与监控主循环

use crate::db::{Db, HardwareRow, ProcessHistoryItem};
use crate::diagnosis::{level_text, AnomalyView, DiagEngine, Marker, SysPoint};
use crate::hardware::{memory_status, CpuSampler, MemoryStatus, StaticInfo};
use crate::pdh::{EngineUsage, PdhCollector};
use crate::process::{ProcessCollector, ProcessInfo};
use crate::sensors::{FanReading, GpuSensor, SensorHub, SensorStatus, TempReading};
use crate::settings::Settings;
use crate::util::{now_ms, round1};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, RwLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CpuState {
    pub usage: f64,
    pub per_core: Vec<f64>,
    pub temp: Option<f64>,
    pub power: Option<f64>,
    pub clock: Option<f64>,
    /// 频率来源：sensor / estimated
    pub clock_source: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuState {
    pub name: String,
    pub vendor: String,
    pub luid: String,
    pub integrated: bool,
    pub usage: Option<f64>,
    pub engines: Vec<(String, f64)>,
    pub temp: Option<f64>,
    pub hotspot: Option<f64>,
    pub power: Option<f64>,
    pub clock: Option<f64>,
    pub mem_clock: Option<f64>,
    pub mem_used: Option<u64>,
    pub mem_total: Option<u64>,
    pub mem_usage: Option<f64>,
    pub shared_used: Option<u64>,
    pub fan_rpm: Option<f64>,
    pub fan_percent: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiskState {
    pub read: Option<f64>,
    pub write: Option<f64>,
    pub active: Option<f64>,
    /// 最热磁盘的温度及名称（传感器不可用时为 None）
    pub temp: Option<f64>,
    pub temp_name: Option<String>,
    /// 每块磁盘的代表温度
    pub temps: Vec<crate::sensors::DiskTemp>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub ts: i64,
    pub cpu: CpuState,
    pub gpus: Vec<GpuState>,
    pub gpu_usage: Option<f64>,
    pub gpu_temp: Option<f64>,
    pub gpu_power: Option<f64>,
    pub gpu_available: bool,
    pub memory: MemoryStatus,
    pub disk: DiskState,
    pub fans: Vec<FanReading>,
    pub fan_rpm: Option<f64>,
    pub temps: Vec<TempReading>,
    pub processes: Vec<ProcessInfo>,
    pub process_count: usize,
    pub thread_count: u64,
    pub handle_count: u64,
    pub anomalies: Vec<AnomalyView>,
    pub level: String,
    pub level_text: String,
    pub sensor: SensorStatus,
    pub hwinfo_mgr: crate::hwinfo_manager::HwinfoMgrStatus,
    pub pawnio: crate::pawnio::PawnioStatus,
    pub new_markers: Vec<Marker>,
    pub point: SysPoint,
}

pub struct AppState {
    pub settings: RwLock<Settings>,
    pub settings_path: PathBuf,
    pub static_info: StaticInfo,
    pub db: Mutex<Option<Db>>,
    pub engine: Mutex<DiagEngine>,
    pub latest: RwLock<Option<Arc<Snapshot>>>,
    pub engines: Mutex<HashMap<u32, Vec<EngineUsage>>>,
    pub sensors: Arc<SensorHub>,
    pub hwinfo_mgr: Arc<crate::hwinfo_manager::HwinfoManager>,
    pub paused: AtomicBool,
    pub wake: (Mutex<bool>, Condvar),
    pub quitting: AtomicBool,
    pub tray_level: Mutex<String>,
}

impl AppState {
    pub fn settings(&self) -> Settings {
        self.settings.read().unwrap().clone()
    }

    pub fn wake_now(&self) {
        let (lock, cv) = &self.wake;
        *lock.lock().unwrap() = true;
        cv.notify_all();
    }

    pub fn with_db<R>(&self, f: impl FnOnce(&Db) -> R) -> Option<R> {
        self.db.lock().unwrap().as_ref().map(f)
    }

    pub fn sensor_notes(&self) -> Vec<String> {
        let st = self.sensors.status();
        let mut notes = vec![];
        if let Some(m) = &st.message {
            notes.push(m.clone());
        }
        if let Some(s) = self.latest.read().unwrap().as_ref() {
            if s.cpu.temp.is_none() {
                notes.push("CPU 温度暂不支持读取".into());
            }
            if s.fan_rpm.is_none() {
                notes.push("风扇转速暂不支持读取（不会显示虚构的转速），诊断将基于温度与负载进行".into());
            }
        }
        notes
    }
}

fn normalize_name(s: &str) -> String {
    s.to_lowercase()
        .replace("(r)", "")
        .replace("(tm)", "")
        .replace("nvidia", "")
        .replace("amd", "")
        .replace("geforce", "")
        .replace("graphics", "")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn vendor_of(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if n.contains("nvidia") || n.contains("geforce") || n.contains("rtx") || n.contains("gtx") || n.contains("quadro") {
        "NVIDIA"
    } else if n.contains("amd") || n.contains("radeon") {
        "AMD"
    } else if n.contains("intel") || n.contains("arc") || n.contains("iris") || n.contains("uhd") {
        "Intel"
    } else {
        "Other"
    }
}

fn match_sensor<'a>(name: &str, vendor: &str, sensors: &'a [GpuSensor], used: &mut Vec<usize>) -> Option<&'a GpuSensor> {
    let n = normalize_name(name);
    let idx = sensors
        .iter()
        .enumerate()
        .filter(|(i, _)| !used.contains(i))
        .find(|(_, s)| {
            let sn = normalize_name(&s.name);
            !sn.is_empty() && !n.is_empty() && (sn.contains(&n) || n.contains(&sn))
        })
        .or_else(|| {
            sensors
                .iter()
                .enumerate()
                .filter(|(i, _)| !used.contains(i))
                .find(|(_, s)| vendor_of(&s.name) == vendor)
        })
        .map(|(i, _)| i)?;
    used.push(idx);
    sensors.get(idx)
}

pub fn start_monitor(app: AppHandle) {
    std::thread::Builder::new()
        .name("monitor".into())
        .spawn(move || monitor_loop(app))
        .expect("无法启动监控线程");
}

fn monitor_loop(app: AppHandle) {
    let state = app.state::<Arc<AppState>>().inner().clone();
    let threads = state.static_info.threads as usize;
    let mut proc_c = ProcessCollector::new(threads);
    let mut cpu_s = CpuSampler::new(threads);
    let mut pdh = PdhCollector::new();
    let mut last_history = 0i64;
    let mut last_cleanup = 0i64;
    let mut last_sensor_interval = 0u64;
    // 预热：首次采样只建立基线
    proc_c.sample();
    std::thread::sleep(Duration::from_millis(300));

    loop {
        let settings = state.settings();
        if settings.sensor_interval_ms != last_sensor_interval {
            state.sensors.set_interval(settings.sensor_interval_ms);
            last_sensor_interval = settings.sensor_interval_ms;
        }
        {
            let (lock, cv) = &state.wake;
            let mut woke = lock.lock().unwrap();
            if !*woke {
                let (g, _) = cv
                    .wait_timeout(woke, Duration::from_millis(settings.process_interval_ms))
                    .unwrap();
                woke = g;
            }
            *woke = false;
        }
        if state.quitting.load(Ordering::SeqCst) {
            break;
        }
        if state.paused.load(Ordering::SeqCst) {
            continue;
        }

        let now = now_ms();
        let (cpu_total, per_core) = cpu_s.sample();
        let mut procs = proc_c.sample();
        let pdh_s = pdh.sample();
        let sensors = state.sensors.summary();
        let mut memory = memory_status().unwrap_or_default();
        memory.pagefile_usage = pdh_s.pagefile_usage;

        // ----- 进程 GPU -----
        if pdh_s.gpu_available {
            for p in procs.iter_mut() {
                p.gpu = Some(round1(*pdh_s.proc_gpu.get(&p.pid).unwrap_or(&0.0)));
                p.gpu_mem = Some(*pdh_s.proc_gpu_mem.get(&p.pid).unwrap_or(&0));
            }
        }
        *state.engines.lock().unwrap() = pdh_s.proc_engines.clone();

        // ----- GPU 汇总 -----
        let mut used = vec![];
        let mut gpus: Vec<GpuState> = state
            .static_info
            .gpus
            .iter()
            .map(|a| {
                let ad = pdh_s.adapters.get(&a.luid);
                let sn = match_sensor(&a.name, &a.vendor, &sensors.gpus, &mut used);
                let usage = if pdh_s.gpu_available {
                    ad.map(|x| round1(x.usage)).or(Some(0.0))
                } else {
                    sn.and_then(|s| s.load)
                };
                let mem_used = ad.and_then(|x| x.dedicated_used).or_else(|| sn.and_then(|s| s.mem_used));
                let mem_total = if a.dedicated_total > 0 {
                    Some(a.dedicated_total)
                } else {
                    sn.and_then(|s| s.mem_total)
                };
                GpuState {
                    name: a.name.clone(),
                    vendor: a.vendor.clone(),
                    luid: a.luid.clone(),
                    integrated: a.integrated,
                    usage,
                    engines: ad.map(|x| x.engines.clone()).unwrap_or_default(),
                    temp: sn.and_then(|s| s.temp),
                    hotspot: sn.and_then(|s| s.hotspot),
                    power: sn.and_then(|s| s.power),
                    clock: sn.and_then(|s| s.clock),
                    mem_clock: sn.and_then(|s| s.mem_clock),
                    mem_used,
                    mem_total,
                    mem_usage: match (mem_used, mem_total) {
                        (Some(u), Some(t)) if t > 0 => Some(round1(u as f64 / t as f64 * 100.0)),
                        _ => None,
                    },
                    shared_used: ad.and_then(|x| x.shared_used),
                    fan_rpm: sn.and_then(|s| s.fan_rpm),
                    fan_percent: sn.and_then(|s| s.fan_percent),
                }
            })
            .collect();
        // DXGI 未枚举到但传感器提供的 GPU
        for (i, s) in sensors.gpus.iter().enumerate() {
            if used.contains(&i) {
                continue;
            }
            gpus.push(GpuState {
                name: s.name.clone(),
                vendor: vendor_of(&s.name).into(),
                luid: String::new(),
                integrated: false,
                usage: s.load,
                engines: vec![],
                temp: s.temp,
                hotspot: s.hotspot,
                power: s.power,
                clock: s.clock,
                mem_clock: s.mem_clock,
                mem_used: s.mem_used,
                mem_total: s.mem_total,
                mem_usage: match (s.mem_used, s.mem_total) {
                    (Some(u), Some(t)) if t > 0 => Some(round1(u as f64 / t as f64 * 100.0)),
                    _ => None,
                },
                shared_used: None,
                fan_rpm: s.fan_rpm,
                fan_percent: s.fan_percent,
            });
        }
        let gpu_usage = gpus.iter().filter_map(|g| g.usage).fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))));
        let gpu_temp = gpus.iter().filter_map(|g| g.temp).fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))));
        let gpu_power = {
            let v: Vec<f64> = gpus.iter().filter_map(|g| g.power).collect();
            if v.is_empty() {
                None
            } else {
                Some(round1(v.iter().sum()))
            }
        };

        // ----- CPU -----
        let (clock, clock_source) = match sensors.cpu_clock {
            Some(c) => (Some(round1(c)), "sensor"),
            None => match (state.static_info.base_mhz, pdh_s.cpu_perf) {
                (Some(b), Some(p)) => (Some(round1(b * p / 100.0)), "estimated"),
                (Some(b), None) => (Some(b), "base"),
                _ => (None, "none"),
            },
        };
        let cpu = CpuState {
            usage: round1(cpu_total),
            per_core,
            temp: sensors.cpu_temp.map(round1),
            power: sensors.cpu_power.map(round1),
            clock,
            clock_source: clock_source.into(),
        };
        let fan_rpm = sensors.main_fan_rpm().map(|v| v.round());
        // 磁盘温度：取所有存储设备温度中的最高值（每块盘可能有多个温度传感器）
        // 先为每块盘选出代表温度（NVMe 优先整盘复合温度，避免主控等单点传感器误报），再取最热的盘
        let disk_temps = crate::sensors::disk_temperatures(&sensors.temps);
        let hot_disk = disk_temps
            .iter()
            .max_by(|a, b| a.value.partial_cmp(&b.value).unwrap_or(std::cmp::Ordering::Equal))
            .map(|d| (round1(d.value), d.hardware.clone()));
        let disk_active = pdh_s.disk_active.map(round1);

        let point = SysPoint {
            ts: now,
            cpu: cpu.usage,
            gpu: gpu_usage,
            mem: round1(memory.usage),
            cpu_temp: cpu.temp,
            gpu_temp,
            cpu_power: cpu.power,
            gpu_power,
            fan: fan_rpm,
            disk_temp: hot_disk.as_ref().map(|d| d.0),
            disk_active,
        };

        // ----- 诊断 -----
        let (new_anomalies, anomalies, level, new_markers) = {
            let db_guard = state.db.lock().unwrap();
            let mut engine = state.engine.lock().unwrap();
            engine.gpu_available = pdh_s.gpu_available || gpu_usage.is_some();
            engine.hot_disk = hot_disk.as_ref().map(|d| d.1.clone());
            let out = engine.update(now, point, &mut procs, &settings, memory.total, db_guard.as_ref());
            let markers: Vec<Marker> = engine.markers().into_iter().filter(|m| m.ts == now).collect();
            (out.new_anomalies, engine.active_views(now), engine.overall_level(), markers)
        };

        // ----- 桌面通知 -----
        if settings.notifications && !new_anomalies.is_empty() {
            use tauri_plugin_notification::NotificationExt;
            for a in &new_anomalies {
                let _ = app
                    .notification()
                    .builder()
                    .title(format!("FanGuard：{}", a.title))
                    .body(a.description.clone())
                    .show();
            }
        }

        // ----- 历史记录 -----
        if now - last_history >= settings.history_interval_ms as i64 {
            last_history = now;
            let mut top: Vec<&ProcessInfo> = procs.iter().filter(|p| p.cpu >= 1.0 || p.gpu.unwrap_or(0.0) >= 1.0 || p.score >= 40.0).collect();
            top.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap_or(std::cmp::Ordering::Equal));
            let items: Vec<ProcessHistoryItem> = top
                .into_iter()
                .take(10)
                .map(|p| ProcessHistoryItem {
                    pid: p.pid,
                    start_time: p.start_time,
                    name: p.name.clone(),
                    cpu: p.cpu,
                    memory: p.memory as f64,
                    gpu: p.gpu,
                })
                .collect();
            if let Some(db) = state.db.lock().unwrap().as_mut() {
                db.insert_hardware(
                    now,
                    &HardwareRow {
                        timestamp: now,
                        cpu_usage: Some(point.cpu),
                        cpu_temp: point.cpu_temp,
                        cpu_power: point.cpu_power,
                        gpu_usage: point.gpu,
                        gpu_temp: point.gpu_temp,
                        gpu_power: point.gpu_power,
                        memory_usage: Some(point.mem),
                        fan_rpm: point.fan,
                        disk_temp: point.disk_temp,
                    },
                );
                db.insert_process_history(now, &items);
                if now - last_cleanup > 3_600_000 {
                    last_cleanup = now;
                    db.cleanup(settings.history_retention_days);
                }
            }
        }

        // ----- 快照 -----
        let thread_count = procs.iter().map(|p| p.threads as u64).sum();
        let handle_count = procs.iter().map(|p| p.handles as u64).sum();
        let snapshot = Arc::new(Snapshot {
            ts: now,
            cpu,
            gpu_available: pdh_s.gpu_available,
            gpus,
            gpu_usage,
            gpu_temp,
            gpu_power,
            memory,
            disk: DiskState {
                read: pdh_s.disk_read,
                write: pdh_s.disk_write,
                active: disk_active,
                temp: hot_disk.as_ref().map(|d| d.0),
                temp_name: hot_disk.map(|d| d.1),
                temps: disk_temps,
            },
            fan_rpm,
            fans: sensors.fans,
            temps: sensors.temps,
            process_count: procs.len(),
            thread_count,
            handle_count,
            processes: procs,
            level_text: level_text(&level).into(),
            level: level.clone(),
            anomalies,
            sensor: state.sensors.status(),
            hwinfo_mgr: state.hwinfo_mgr.status(),
            pawnio: crate::pawnio::status(),
            new_markers,
            point,
        });
        *state.latest.write().unwrap() = Some(snapshot.clone());

        crate::tray::update(&app, &state, &snapshot);

        // 窗口隐藏时不推送，降低后台资源占用；窗口显示时前端会重新同步
        let visible = app
            .get_webview_window("main")
            .and_then(|w| w.is_visible().ok())
            .unwrap_or(false);
        if visible {
            let _ = app.emit_to("main", "snapshot", &*snapshot);
        }
    }
}
