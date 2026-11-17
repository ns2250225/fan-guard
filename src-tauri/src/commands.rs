//! Tauri 命令（前端 IPC 接口）

use crate::db::{ActionRow, EventRow, HardwareRow, ProcessAggRow};
use crate::diagnosis::{DiagnosisReport, Marker, ScoreBreakdown, Suspect, SysPoint};
use crate::hardware::StaticInfo;
use crate::monitor::{AppState, Snapshot};
use crate::process::{ProcKind, ProcessInfo, RawThread};
use crate::process_ops::{self, FileInfo, KillMode, KillPreview, KillResult, SignatureInfo};
use crate::sensors::{HwNode, SensorStatus};
use crate::settings::Settings;
use crate::util::now_ms;
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, State};

type St<'a> = State<'a, Arc<AppState>>;

#[tauri::command]
pub fn get_static_info(state: St) -> StaticInfo {
    state.static_info.clone()
}

#[tauri::command]
pub fn get_snapshot(state: St) -> Option<Snapshot> {
    state.latest.read().unwrap().as_ref().map(|s| (**s).clone())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Realtime {
    pub points: Vec<SysPoint>,
    pub markers: Vec<Marker>,
}

#[tauri::command]
pub fn get_realtime(state: St) -> Realtime {
    let e = state.engine.lock().unwrap();
    Realtime {
        points: e.sys_points(),
        markers: e.markers(),
    }
}

#[tauri::command]
pub fn get_settings(state: St) -> Settings {
    state.settings()
}

#[tauri::command]
pub fn save_settings(app: AppHandle, state: St, settings: Settings) -> Result<Settings, String> {
    let mut s = settings;
    s.sanitize();
    let old = state.settings();
    if s.autostart != old.autostart {
        use tauri_plugin_autostart::ManagerExt;
        let al = app.autolaunch();
        let r = if s.autostart { al.enable() } else { al.disable() };
        if let Err(e) = r {
            return Err(format!("设置开机启动失败：{}", e));
        }
    }
    s.save(&state.settings_path)?;
    *state.settings.write().unwrap() = s.clone();
    if s.sensor_interval_ms != old.sensor_interval_ms {
        state.sensors.set_interval(s.sensor_interval_ms);
    }
    if s.sensor_source != old.sensor_source {
        state.sensors.set_source(&s.sensor_source);
    }
    if s.hwinfo_auto != old.hwinfo_auto {
        state.hwinfo_mgr.set_auto(s.hwinfo_auto);
    }
    state.wake_now();
    Ok(s)
}

#[tauri::command]
pub fn set_whitelist(state: St, name: String, add: bool) -> Result<Settings, String> {
    let mut s = state.settings();
    if add {
        if !s.is_whitelisted(&name) {
            s.whitelist.push(name);
        }
    } else {
        s.whitelist.retain(|w| !w.eq_ignore_ascii_case(&name));
    }
    s.sanitize();
    s.save(&state.settings_path)?;
    *state.settings.write().unwrap() = s.clone();
    Ok(s)
}

#[tauri::command]
pub fn set_paused(app: AppHandle, paused: bool) {
    crate::tray::set_paused(&app, paused);
}

#[tauri::command]
pub fn get_paused(state: St) -> bool {
    state.paused.load(Ordering::SeqCst)
}

#[tauri::command]
pub fn refresh_now(state: St) {
    state.wake_now();
}

// ---------------------------------------------------------------------------
// 进程
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcBrief {
    pub pid: u32,
    #[serde(serialize_with = "crate::util::ser_i64_str")]
    pub create_time: i64,
    pub name: String,
    pub cpu: f64,
    pub memory: u64,
    pub gpu: Option<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineView {
    pub adapter: String,
    pub engine: String,
    pub usage: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessDetail {
    pub info: ProcessInfo,
    pub alive: bool,
    pub parent: Option<ProcBrief>,
    pub children: Vec<ProcBrief>,
    /// [时间戳, CPU%, GPU%, 内存字节]
    pub history: Vec<(i64, f64, f64, u64)>,
    pub gpu_engines: Vec<EngineView>,
    pub gpu_supported: bool,
    pub score: f64,
    pub breakdown: Option<ScoreBreakdown>,
    pub suspect: Option<Suspect>,
    pub file: Option<FileInfo>,
    pub kind_text: String,
    pub elevated: bool,
    pub logical_processors: u32,
}

fn kind_text(k: ProcKind) -> &'static str {
    match k {
        ProcKind::Critical => "关键系统进程（受保护）",
        ProcKind::Service => "系统服务 / 后台服务",
        ProcKind::System => "Windows 系统组件",
        ProcKind::User => "用户进程",
    }
}

#[tauri::command]
pub fn get_process_detail(state: St, pid: u32, create_time: String, with_file: bool) -> Result<ProcessDetail, String> {
    let create_time = crate::util::parse_create_time(&create_time)?;
    let snap = state.latest.read().unwrap().clone().ok_or("监控数据尚未就绪")?;
    let found = snap
        .processes
        .iter()
        .find(|p| p.pid == pid && p.create_time == create_time)
        .cloned();
    let (info, alive) = match found {
        Some(i) => (i, true),
        None => {
            // 进程已退出：返回最后已知的名字
            let e = state.engine.lock().unwrap();
            let name = e.process_name((pid, create_time)).ok_or("进程已经退出")?;
            drop(e);
            (
                ProcessInfo {
                    pid,
                    ppid: 0,
                    name,
                    create_time,
                    start_time: crate::util::filetime_to_unix_ms(create_time),
                    cpu: 0.0,
                    memory: 0,
                    working_set: 0,
                    private_bytes: 0,
                    gpu: None,
                    gpu_mem: None,
                    io_read: 0.0,
                    io_write: 0.0,
                    threads: 0,
                    handles: 0,
                    session_id: 0,
                    path: None,
                    kind: ProcKind::User,
                    status: "exited".into(),
                    score: 0.0,
                    whitelisted: false,
                    ignored: false,
                },
                false,
            )
        }
    };
    let parent = snap
        .processes
        .iter()
        .find(|p| p.pid == info.ppid && p.create_time <= info.create_time && info.ppid != 0)
        .map(|p| ProcBrief {
            pid: p.pid,
            create_time: p.create_time,
            name: p.name.clone(),
            cpu: p.cpu,
            memory: p.memory,
            gpu: p.gpu,
        });
    let mut children: Vec<ProcBrief> = snap
        .processes
        .iter()
        .filter(|p| p.ppid == pid && p.create_time >= create_time && p.pid != pid)
        .map(|p| ProcBrief {
            pid: p.pid,
            create_time: p.create_time,
            name: p.name.clone(),
            cpu: p.cpu,
            memory: p.memory,
            gpu: p.gpu,
        })
        .collect();
    children.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap_or(std::cmp::Ordering::Equal));

    let adapters = &state.static_info.gpus;
    let gpu_engines = state
        .engines
        .lock()
        .unwrap()
        .get(&pid)
        .map(|list| {
            let mut v: Vec<EngineView> = list
                .iter()
                .map(|e| EngineView {
                    adapter: adapters
                        .iter()
                        .find(|a| a.luid == e.luid)
                        .map(|a| a.name.clone())
                        .unwrap_or_else(|| e.luid.clone()),
                    engine: e.engine.clone(),
                    usage: e.usage,
                })
                .collect();
            v.sort_by(|a, b| b.usage.partial_cmp(&a.usage).unwrap_or(std::cmp::Ordering::Equal));
            v
        })
        .unwrap_or_default();

    let settings = state.settings();
    let (history, breakdown, suspect) = {
        let e = state.engine.lock().unwrap();
        let h = e.process_history((pid, create_time));
        let b = e.process_breakdown((pid, create_time));
        let s = if alive {
            Some(e.make_suspect(&info, &settings, state.static_info.total_memory))
        } else {
            None
        };
        (h, b, s)
    };
    let file = if with_file {
        info.path.as_deref().map(process_ops::file_info)
    } else {
        None
    };
    Ok(ProcessDetail {
        kind_text: kind_text(info.kind).into(),
        score: breakdown.as_ref().map(|b| b.0).unwrap_or(info.score),
        breakdown: breakdown.map(|b| b.1),
        info,
        alive,
        parent,
        children,
        history,
        gpu_engines,
        gpu_supported: snap.gpu_available,
        suspect,
        file,
        elevated: process_ops::is_elevated(),
        logical_processors: state.static_info.threads,
    })
}

#[tauri::command]
pub fn get_process_threads(pid: u32, create_time: String) -> Result<Vec<RawThread>, String> {
    let create_time = crate::util::parse_create_time(&create_time)?;
    process_ops::threads(pid, create_time)
}

#[tauri::command]
pub async fn get_signature(path: String) -> SignatureInfo {
    tauri::async_runtime::spawn_blocking(move || process_ops::signature(&path))
        .await
        .unwrap_or_default()
}

#[tauri::command]
pub fn get_file_info(path: String) -> FileInfo {
    process_ops::file_info(&path)
}

#[tauri::command]
pub fn open_file_location(path: String) -> Result<(), String> {
    process_ops::open_file_location(&path)
}

#[tauri::command]
pub fn show_file_properties(path: String) -> Result<(), String> {
    process_ops::show_file_properties(&path)
}

#[tauri::command]
pub fn preview_kill(pid: u32, create_time: String) -> Result<KillPreview, String> {
    let create_time = crate::util::parse_create_time(&create_time)?;
    process_ops::preview(pid, create_time)
}

#[tauri::command]
pub async fn kill_process(state: St<'_>, pid: u32, create_time: String, mode: String) -> Result<KillResult, String> {
    let create_time = crate::util::parse_create_time(&create_time)?;
    let mode = KillMode::parse(&mode).ok_or("未知的操作类型")?;
    let st = state.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || process_ops::kill(pid, create_time, mode))
        .await
        .map_err(|e| e.to_string())?;
    // 记录操作
    let summary = if result.success {
        format!("成功：{}", result.message)
    } else {
        format!("失败：{}", result.message)
    };
    let name = if result.process_name.is_empty() {
        st.engine
            .lock()
            .unwrap()
            .process_name((pid, create_time))
            .unwrap_or_else(|| format!("PID {}", pid))
    } else {
        result.process_name.clone()
    };
    st.with_db(|db| db.insert_action(now_ms(), pid, &name, mode.label(), &summary));
    let _ = mode.code();
    st.wake_now();
    Ok(result)
}

// ---------------------------------------------------------------------------
// 诊断
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn run_diagnosis(state: St) -> Result<DiagnosisReport, String> {
    let snap = state.latest.read().unwrap().clone().ok_or("监控数据尚未就绪，请稍候")?;
    let settings = state.settings();
    let notes = state.sensor_notes();
    let e = state.engine.lock().unwrap();
    Ok(e.report(
        now_ms(),
        &snap.processes,
        &settings,
        notes,
        state.static_info.total_memory,
        snap.memory.usage,
    ))
}

#[tauri::command]
pub fn ignore_anomalies(state: St, code: Option<String>) {
    let db = state.db.lock().unwrap();
    let mut e = state.engine.lock().unwrap();
    match code {
        Some(c) => e.ignore(&c, db.as_ref()),
        None => {
            e.ignore_all(db.as_ref());
        }
    }
}

// ---------------------------------------------------------------------------
// 历史
// ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryData {
    pub from: i64,
    pub to: i64,
    pub hardware: Vec<HardwareRow>,
    pub events: Vec<EventRow>,
    pub actions: Vec<ActionRow>,
    pub processes: Vec<ProcessAggRow>,
}

fn range_ms(range: &str) -> i64 {
    match range {
        "1h" => 3_600_000,
        "24h" => 86_400_000,
        "7d" => 7 * 86_400_000,
        "30d" => 30 * 86_400_000,
        _ => 3_600_000,
    }
}

#[tauri::command]
pub fn get_history(state: St, range: String) -> HistoryData {
    let to = now_ms();
    let from = to - range_ms(&range);
    state
        .with_db(|db| HistoryData {
            from,
            to,
            hardware: db.query_hardware(from, to, 720),
            events: db.query_events(from, to, 500),
            actions: db.query_actions(from, to, 500),
            processes: db.query_process_agg(from, to, 20),
        })
        .unwrap_or(HistoryData {
            from,
            to,
            hardware: vec![],
            events: vec![],
            actions: vec![],
            processes: vec![],
        })
}

#[tauri::command]
pub fn export_csv(state: St, kind: String, range: String, path: String) -> Result<usize, String> {
    let to = now_ms();
    let from = to - range_ms(&range);
    state
        .with_db(|db| db.export_csv(&kind, from, to, std::path::Path::new(&path)))
        .unwrap_or(Err("数据库不可用".into()))
}

#[tauri::command]
pub fn clear_history(state: St) -> Result<(), String> {
    state.with_db(|db| db.clear_all()).unwrap_or(Err("数据库不可用".into()))
}

// ---------------------------------------------------------------------------
// 传感器 / 系统
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_sensor_raw(state: St) -> Vec<HwNode> {
    state.sensors.raw()
}

#[tauri::command]
pub fn get_sensor_status(state: St) -> SensorStatus {
    state.sensors.status()
}

#[tauri::command]
pub fn is_elevated() -> bool {
    process_ops::is_elevated()
}

#[tauri::command]
pub fn relaunch_admin(app: AppHandle) -> Result<(), String> {
    process_ops::relaunch_as_admin()?;
    crate::quit(&app);
    Ok(())
}

#[tauri::command]
pub fn quit_app(app: AppHandle) {
    crate::quit(&app);
}

#[tauri::command]
pub fn hide_to_tray(app: AppHandle) {
    use tauri::Manager;
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.hide();
    }
}

#[tauri::command]
pub fn close_window(app: AppHandle, state: St) {
    use tauri::Manager;
    if state.settings().minimize_to_tray {
        if let Some(w) = app.get_webview_window("main") {
            let _ = w.hide();
        }
    } else {
        crate::quit(&app);
    }
}

// ---------------------------------------------------------------------------
// HWiNFO 集成
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn hwinfo_launch(state: St) {
    state.hwinfo_mgr.request_launch();
}

#[tauri::command]
pub async fn hwinfo_import(state: St<'_>, path: String) -> Result<String, String> {
    let mgr = state.hwinfo_mgr.clone();
    let p = tauri::async_runtime::spawn_blocking(move || mgr.import(std::path::Path::new(&path)))
        .await
        .map_err(|e| e.to_string())??;
    state.hwinfo_mgr.request_launch();
    Ok(p.display().to_string())
}

#[tauri::command]
pub fn hwinfo_open_download() -> Result<(), String> {
    crate::hwinfo_manager::open_download_page()
}
