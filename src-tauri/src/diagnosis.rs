//! 智能风扇诊断引擎
//!
//! - 以时间窗口统计每个进程的 CPU / GPU / 内存，而不是依据单次瞬时值；
//! - 计算"异常关联分数"（优先排查分数，不是真实的风扇因果贡献率）；
//! - 检测系统级异常：CPU/GPU 持续高负载、高温、风扇高速、风扇高速但负载低；
//! - 生成诊断报告与优化建议。

use crate::db::Db;
use crate::process::{ProcKey, ProcKind, ProcessInfo};
use crate::settings::Settings;
use crate::util::{format_bytes, round1};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};

/// 进程样本保留时长（毫秒）——同时用于进程详情中的 CPU 历史曲线
const PROC_KEEP_MS: i64 = 300_000;
/// 系统样本保留时长
const SYS_KEEP_MS: i64 = 900_000;

#[derive(Debug, Clone, Copy)]
struct ProcSample {
    ts: i64,
    cpu: f32,
    gpu: f32,
    mem: u64,
}

struct ProcWindow {
    name: String,
    samples: VecDeque<ProcSample>,
    high_since: Option<i64>,
    last_seen: i64,
    breakdown: ScoreBreakdown,
    score: f64,
    suspect_event: Option<i64>,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SysPoint {
    pub ts: i64,
    pub cpu: f64,
    pub gpu: Option<f64>,
    pub mem: f64,
    pub cpu_temp: Option<f64>,
    pub gpu_temp: Option<f64>,
    pub cpu_power: Option<f64>,
    pub gpu_power: Option<f64>,
    pub fan: Option<f64>,
    /// 最热磁盘的温度
    pub disk_temp: Option<f64>,
    /// 磁盘活动时间（%）
    pub disk_active: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreBreakdown {
    pub cpu: f64,
    pub gpu: f64,
    pub memory: f64,
    pub power: f64,
    pub sustain: f64,
    /// 各维度的有效权重（数据不可用时为 0）
    pub weights: [f64; 5],
    /// power / temperature / unavailable
    pub power_source: String,
    pub gpu_available: bool,
    pub avg_cpu: f64,
    pub avg_gpu: f64,
    pub mem_growth: i64,
    pub sustained_secs: i64,
    pub window_secs: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AnomalyKind {
    CpuHigh,
    GpuHigh,
    CpuTemp,
    GpuTemp,
    FanHigh,
    FanHighLowLoad,
    TempHighLowLoad,
    DiskTemp,
}

impl AnomalyKind {
    pub fn title(&self) -> &'static str {
        match self {
            Self::CpuHigh => "CPU 持续高负载",
            Self::GpuHigh => "GPU 持续高负载",
            Self::CpuTemp => "CPU 温度过高",
            Self::GpuTemp => "GPU 温度过高",
            Self::FanHigh => "风扇转速异常升高",
            Self::FanHighLowLoad => "风扇高速旋转但负载较低",
            Self::TempHighLowLoad => "负载正常但温度偏高",
            Self::DiskTemp => "磁盘温度过高",
        }
    }
    pub fn code(&self) -> &'static str {
        match self {
            Self::CpuHigh => "cpu_high",
            Self::GpuHigh => "gpu_high",
            Self::CpuTemp => "cpu_temp",
            Self::GpuTemp => "gpu_temp",
            Self::FanHigh => "fan_high",
            Self::FanHighLowLoad => "fan_high_low_load",
            Self::TempHighLowLoad => "temp_high_low_load",
            Self::DiskTemp => "disk_temp",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnomalyView {
    pub kind: AnomalyKind,
    pub code: String,
    pub title: String,
    /// notice / high / critical
    pub level: String,
    pub since: i64,
    pub duration_secs: i64,
    pub value: Option<f64>,
    pub threshold: Option<f64>,
    pub description: String,
    pub ignored: bool,
    pub related_pid: Option<u32>,
    pub related_name: Option<String>,
}

struct ActiveAnomaly {
    kind: AnomalyKind,
    db_id: i64,
    since: i64,
    level: String,
    value: Option<f64>,
    threshold: Option<f64>,
    description: String,
    ignored: bool,
    related: Option<(u32, String)>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Marker {
    pub ts: i64,
    pub title: String,
    pub level: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suspect {
    pub pid: u32,
    #[serde(serialize_with = "crate::util::ser_i64_str")]
    pub create_time: i64,
    pub name: String,
    pub path: Option<String>,
    pub cpu_avg: f64,
    pub gpu_avg: Option<f64>,
    pub memory: u64,
    pub duration_secs: i64,
    pub score: f64,
    pub grade: String,
    pub grade_text: String,
    pub correlation: String,
    pub reasons: Vec<String>,
    pub breakdown: ScoreBreakdown,
    pub protected: bool,
    pub whitelisted: bool,
    pub kind: ProcKind,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Suggestion {
    pub title: String,
    pub detail: String,
    pub related_pid: Option<u32>,
    pub related_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosisReport {
    pub generated_at: i64,
    pub level: String,
    pub level_text: String,
    pub title: String,
    pub duration_secs: Option<i64>,
    pub anomalies: Vec<AnomalyView>,
    pub suspects: Vec<Suspect>,
    pub hardware: SysPoint,
    pub reasons: Vec<String>,
    pub suggestions: Vec<Suggestion>,
    pub data_notes: Vec<String>,
    pub window_secs: i64,
}

pub struct EngineOutput {
    pub new_anomalies: Vec<AnomalyView>,
}

pub struct DiagEngine {
    procs: HashMap<ProcKey, ProcWindow>,
    sys: VecDeque<SysPoint>,
    active: HashMap<AnomalyKind, ActiveAnomaly>,
    /// 风扇基线（低负载期间的平滑转速）
    fan_baseline: Option<f64>,
    markers: VecDeque<Marker>,
    pub gpu_available: bool,
    /// 当前温度最高的磁盘名称（由监控循环设置）
    pub hot_disk: Option<String>,
}

pub fn grade_of(score: f64) -> (&'static str, &'static str) {
    if score >= 80.0 {
        ("critical", "重点排查")
    } else if score >= 60.0 {
        ("high", "高负载")
    } else if score >= 40.0 {
        ("attention", "需要关注")
    } else {
        ("normal", "正常")
    }
}

pub fn level_text(level: &str) -> &'static str {
    match level {
        "critical" => "严重",
        "high" => "高",
        "notice" => "注意",
        _ => "正常",
    }
}

fn level_rank(level: &str) -> u8 {
    match level {
        "critical" => 3,
        "high" => 2,
        "notice" => 1,
        _ => 0,
    }
}

fn pearson(xs: &[f64], ys: &[f64]) -> Option<f64> {
    let n = xs.len().min(ys.len());
    if n < 5 {
        return None;
    }
    let mx = xs[..n].iter().sum::<f64>() / n as f64;
    let my = ys[..n].iter().sum::<f64>() / n as f64;
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    let mut syy = 0.0;
    for i in 0..n {
        let dx = xs[i] - mx;
        let dy = ys[i] - my;
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    if sxx < 1e-9 || syy < 1e-9 {
        return None;
    }
    Some(sxy / (sxx.sqrt() * syy.sqrt()))
}

fn avg_opt<I: Iterator<Item = Option<f64>>>(it: I) -> Option<f64> {
    let mut sum = 0.0;
    let mut n = 0usize;
    for v in it.flatten() {
        sum += v;
        n += 1;
    }
    if n == 0 {
        None
    } else {
        Some(sum / n as f64)
    }
}

impl DiagEngine {
    pub fn new() -> Self {
        Self {
            procs: HashMap::new(),
            sys: VecDeque::new(),
            active: HashMap::new(),
            fan_baseline: None,
            markers: VecDeque::new(),
            gpu_available: false,
            hot_disk: None,
        }
    }

    fn window_ms(settings: &Settings) -> i64 {
        (settings.sustain_seconds.max(10) as i64) * 1000
    }

    /// 每个采样周期调用一次
    pub fn update(
        &mut self,
        now: i64,
        point: SysPoint,
        procs: &mut [ProcessInfo],
        settings: &Settings,
        total_memory: u64,
        db: Option<&Db>,
    ) -> EngineOutput {
        // ---------- 系统样本 ----------
        self.sys.push_back(point);
        while self.sys.front().map_or(false, |p| now - p.ts > SYS_KEEP_MS) {
            self.sys.pop_front();
        }
        let window = Self::window_ms(settings);
        let sys_win: Vec<SysPoint> = self.sys.iter().filter(|p| now - p.ts <= window).copied().collect();

        let power_series_cpu: Vec<Option<f64>> = sys_win.iter().map(|p| p.cpu_power).collect();
        let power_series_gpu: Vec<Option<f64>> = sys_win.iter().map(|p| p.gpu_power).collect();
        let temp_series_cpu: Vec<Option<f64>> = sys_win.iter().map(|p| p.cpu_temp).collect();
        let temp_series_gpu: Vec<Option<f64>> = sys_win.iter().map(|p| p.gpu_temp).collect();
        let has = |s: &Vec<Option<f64>>| s.iter().filter(|v| v.is_some()).count() >= 5;
        let power_ok = has(&power_series_cpu) || has(&power_series_gpu);
        let temp_ok = has(&temp_series_cpu) || has(&temp_series_gpu);
        let sys_ts: Vec<i64> = sys_win.iter().map(|p| p.ts).collect();
        let sys_cpu_avg = sys_win.iter().map(|p| p.cpu).sum::<f64>() / sys_win.len().max(1) as f64;
        let sys_gpu_avg = avg_opt(sys_win.iter().map(|p| p.gpu)).unwrap_or(0.0);

        // ---------- 进程窗口与评分 ----------
        let mut alive: HashSet<ProcKey> = HashSet::with_capacity(procs.len());
        let gpu_available = self.gpu_available;
        for p in procs.iter_mut() {
            let key = (p.pid, p.create_time);
            alive.insert(key);
            let w = self.procs.entry(key).or_insert_with(|| ProcWindow {
                name: p.name.clone(),
                samples: VecDeque::with_capacity(64),
                high_since: None,
                last_seen: now,
                breakdown: ScoreBreakdown::default(),
                score: 0.0,
                suspect_event: None,
            });
            w.last_seen = now;
            p.whitelisted = settings.is_whitelisted(&p.name);
            p.ignored = settings.is_ignored(&p.name);
            w.samples.push_back(ProcSample {
                ts: now,
                cpu: p.cpu as f32,
                gpu: p.gpu.unwrap_or(0.0) as f32,
                mem: p.memory,
            });
            while w.samples.front().map_or(false, |s| now - s.ts > PROC_KEEP_MS) {
                w.samples.pop_front();
            }
            let high_now = p.cpu >= 10.0 || p.gpu.unwrap_or(0.0) >= 20.0;
            if high_now {
                w.high_since.get_or_insert(now);
            } else {
                // 允许短暂回落（低于阈值的一半才视为中断）
                if p.cpu < 5.0 && p.gpu.unwrap_or(0.0) < 10.0 {
                    w.high_since = None;
                }
            }

            let win: Vec<&ProcSample> = w.samples.iter().filter(|s| now - s.ts <= window).collect();
            let span = win.last().map(|l| l.ts).unwrap_or(now) - win.first().map(|f| f.ts).unwrap_or(now);
            if win.len() < 5 || span < 4000 {
                // 样本不足：不依据瞬时值评分
                p.score = 0.0;
                w.score = 0.0;
                continue;
            }
            let avg_cpu = win.iter().map(|s| s.cpu as f64).sum::<f64>() / win.len() as f64;
            let avg_gpu = win.iter().map(|s| s.gpu as f64).sum::<f64>() / win.len() as f64;
            let mem_now = p.memory as f64;
            let mem_growth = win.last().unwrap().mem as i64 - win.first().unwrap().mem as i64;

            let s_cpu = (avg_cpu / 40.0).min(1.0);
            let s_gpu = (avg_gpu / 60.0).min(1.0);
            let s_mem = 0.5 * (mem_now / (total_memory.max(1) as f64 * 0.2)).min(1.0)
                + 0.5 * (mem_growth.max(0) as f64 / (512.0 * 1024.0 * 1024.0)).min(1.0);

            // 功耗关联：进程负载序列与功耗序列的皮尔逊相关系数 × 负载占比
            let mut power_source = "unavailable";
            let mut s_power = 0.0;
            let mut w_power = 0.0;
            if avg_cpu >= 0.5 || avg_gpu >= 0.5 {
                let by_ts: HashMap<i64, &ProcSample> = win.iter().map(|s| (s.ts, *s)).collect();
                let corr = |series: &Vec<Option<f64>>, use_gpu: bool| -> Option<f64> {
                    let mut xs = vec![];
                    let mut ys = vec![];
                    for (i, ts) in sys_ts.iter().enumerate() {
                        if let (Some(v), Some(s)) = (series[i], by_ts.get(ts)) {
                            xs.push(if use_gpu { s.gpu as f64 } else { s.cpu as f64 });
                            ys.push(v);
                        }
                    }
                    pearson(&xs, &ys)
                };
                let (cs, gs) = if power_ok {
                    power_source = "power";
                    (&power_series_cpu, &power_series_gpu)
                } else if temp_ok {
                    power_source = "temperature";
                    (&temp_series_cpu, &temp_series_gpu)
                } else {
                    (&power_series_cpu, &power_series_gpu)
                };
                if power_source != "unavailable" {
                    let share_cpu = (avg_cpu / sys_cpu_avg.max(1.0)).min(1.0);
                    let share_gpu = (avg_gpu / sys_gpu_avg.max(1.0)).min(1.0);
                    let c1 = corr(cs, false).unwrap_or(0.0).max(0.0) * (share_cpu * 1.5).min(1.0);
                    let c2 = if gpu_available {
                        corr(gs, true).unwrap_or(0.0).max(0.0) * (share_gpu * 1.5).min(1.0)
                    } else {
                        0.0
                    };
                    // 若系统整体负载变化很小，相关系数不稳定，采用负载占比作为下限估计
                    let base = if avg_cpu >= 5.0 { share_cpu * 0.5 } else { 0.0 };
                    s_power = c1.max(c2).max(base).min(1.0);
                }
            } else if power_ok {
                power_source = "power";
            } else if temp_ok {
                power_source = "temperature";
            }
            if power_source == "power" {
                w_power = 20.0;
            } else if power_source == "temperature" {
                // 功耗不可用时以温度变化估算，权重减半
                w_power = 10.0;
            }

            let sustained_secs = w.high_since.map(|t| (now - t) / 1000).unwrap_or(0);
            let s_sustain = (sustained_secs as f64 / 180.0).min(1.0);

            let w_gpu = if gpu_available { 25.0 } else { 0.0 };
            let weights = [35.0, w_gpu, 10.0, w_power, 10.0];
            let total_w: f64 = weights.iter().sum();
            let raw = weights[0] * s_cpu + weights[1] * s_gpu + weights[2] * s_mem + weights[3] * s_power + weights[4] * s_sustain;
            let mut score = (raw / total_w * 100.0).clamp(0.0, 100.0);
            // 进程几乎没有 CPU/GPU 负载时，仅内存占用不足以构成风扇狂转原因
            if avg_cpu < 2.0 && avg_gpu < 5.0 {
                score = score.min(25.0);
            }
            let score = round1(score);
            w.breakdown = ScoreBreakdown {
                cpu: round1(s_cpu * 100.0),
                gpu: round1(s_gpu * 100.0),
                memory: round1(s_mem * 100.0),
                power: round1(s_power * 100.0),
                sustain: round1(s_sustain * 100.0),
                weights,
                power_source: power_source.into(),
                gpu_available,
                avg_cpu: round1(avg_cpu),
                avg_gpu: round1(avg_gpu),
                mem_growth,
                sustained_secs,
                window_secs: window / 1000,
            };
            w.score = score;
            p.score = score;
            if p.kind != ProcKind::Critical {
                p.status = if score >= 60.0 && !p.whitelisted && !p.ignored {
                    "abnormal".into()
                } else {
                    "normal".into()
                };
            }

            // 异常进程事件记录
            if let Some(db) = db {
                if !p.whitelisted && !p.ignored {
                    if score >= 60.0 && w.suspect_event.is_none() {
                        let (_, gt) = grade_of(score);
                        let desc = format!(
                            "{} 被标记为「{}」：分数 {:.0}，平均 CPU {:.1}%{}，内存 {}",
                            p.name,
                            gt,
                            score,
                            avg_cpu,
                            if gpu_available { format!("，平均 GPU {:.1}%", avg_gpu) } else { String::new() },
                            format_bytes(mem_now)
                        );
                        let level = if score >= 80.0 { "high" } else { "notice" };
                        w.suspect_event = Some(db.insert_event(now, level, "异常进程", &desc, Some(p.pid), Some(&p.name), "active"));
                    } else if score < 40.0 {
                        if let Some(id) = w.suspect_event.take() {
                            db.update_event(id, "resolved", Some(now), None, None);
                        }
                    }
                }
            }
        }
        // 已退出的进程
        let mut exited_events = vec![];
        self.procs.retain(|k, w| {
            let keep = alive.contains(k);
            if !keep {
                if let Some(id) = w.suspect_event {
                    exited_events.push(id);
                }
            }
            keep
        });
        if let Some(db) = db {
            for id in exited_events {
                db.update_event(id, "resolved", Some(now), None, None);
            }
        }

        // ---------- 系统级异常检测 ----------
        let mut detections: Vec<(AnomalyKind, bool, String, Option<f64>, Option<f64>, String)> = vec![];
        let covered = sys_win.first().map_or(0, |f| now - f.ts) >= (window as f64 * 0.8) as i64;
        let short: Vec<&SysPoint> = self.sys.iter().filter(|p| now - p.ts <= 10_000).collect();
        let short_avg = |f: &dyn Fn(&SysPoint) -> Option<f64>| avg_opt(short.iter().map(|p| f(p)));

        let top = self.top_suspect(procs, settings);
        let top_text = top
            .as_ref()
            .map(|(_, n, c)| format!("，主要关联进程：{}（CPU {:.0}%）", n, c))
            .unwrap_or_default();

        // CPU 持续高负载
        {
            let active = self.active.contains_key(&AnomalyKind::CpuHigh);
            let thr = settings.cpu_threshold;
            let on = covered && sys_cpu_avg >= thr;
            let keep = active && sys_cpu_avg >= thr - 5.0;
            let temp_hot = point.cpu_temp.map_or(false, |t| t >= settings.cpu_temp_threshold);
            let level = if sys_cpu_avg >= 95.0 && temp_hot { "critical" } else { "high" };
            detections.push((
                AnomalyKind::CpuHigh,
                on || keep,
                format!("最近 {} 秒 CPU 平均使用率 {:.0}%，超过阈值 {:.0}%{}", window / 1000, sys_cpu_avg, thr, top_text),
                Some(round1(sys_cpu_avg)),
                Some(thr),
                level.into(),
            ));
        }
        // GPU 持续高负载
        if self.gpu_available {
            let active = self.active.contains_key(&AnomalyKind::GpuHigh);
            let thr = settings.gpu_threshold;
            let on = covered && sys_gpu_avg >= thr;
            let keep = active && sys_gpu_avg >= thr - 5.0;
            let gtop = procs
                .iter()
                .filter(|p| !settings.is_ignored(&p.name))
                .max_by(|a, b| a.gpu.unwrap_or(0.0).partial_cmp(&b.gpu.unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal))
                .filter(|p| p.gpu.unwrap_or(0.0) > 5.0)
                .map(|p| format!("，主要关联进程：{}（GPU {:.0}%）", p.name, p.gpu.unwrap_or(0.0)))
                .unwrap_or_default();
            detections.push((
                AnomalyKind::GpuHigh,
                on || keep,
                format!("最近 {} 秒 GPU 平均使用率 {:.0}%，超过阈值 {:.0}%{}", window / 1000, sys_gpu_avg, thr, gtop),
                Some(round1(sys_gpu_avg)),
                Some(thr),
                "high".into(),
            ));
        }
        // CPU 高温
        if let Some(t) = short_avg(&|p| p.cpu_temp) {
            let active = self.active.contains_key(&AnomalyKind::CpuTemp);
            let thr = settings.cpu_temp_threshold;
            let on = short.len() >= 3 && t >= thr;
            let keep = active && t >= thr - 3.0;
            let level = if t >= thr + 5.0 { "critical" } else { "high" };
            detections.push((
                AnomalyKind::CpuTemp,
                on || keep,
                format!("CPU 温度 {:.0}°C，超过告警阈值 {:.0}°C，正在检查 CPU 负载来源{}", t, thr, top_text),
                Some(round1(t)),
                Some(thr),
                level.into(),
            ));
            // 负载正常但温度偏高
            let active2 = self.active.contains_key(&AnomalyKind::TempHighLowLoad);
            let near = t >= thr - 5.0;
            let low_load = covered && sys_cpu_avg < 30.0 && sys_gpu_avg < 30.0;
            detections.push((
                AnomalyKind::TempHighLowLoad,
                (near && low_load) || (active2 && t >= thr - 8.0 && sys_cpu_avg < 40.0),
                format!("CPU 负载仅 {:.0}%，但温度达到 {:.0}°C，可能存在散热问题", sys_cpu_avg, t),
                Some(round1(t)),
                Some(thr - 5.0),
                "notice".into(),
            ));
        }
        // GPU 高温
        if let Some(t) = short_avg(&|p| p.gpu_temp) {
            let active = self.active.contains_key(&AnomalyKind::GpuTemp);
            let thr = settings.gpu_temp_threshold;
            let on = short.len() >= 3 && t >= thr;
            let keep = active && t >= thr - 3.0;
            let level = if t >= thr + 5.0 { "critical" } else { "high" };
            detections.push((
                AnomalyKind::GpuTemp,
                on || keep,
                format!("GPU 温度 {:.0}°C，超过告警阈值 {:.0}°C，正在检查 GPU 负载来源", t, thr),
                Some(round1(t)),
                Some(thr),
                level.into(),
            ));
        }
        // 磁盘高温：关联读写量最大的进程
        let io_top = procs
            .iter()
            .filter(|p| !settings.is_ignored(&p.name))
            .max_by(|a, b| {
                (a.io_read + a.io_write)
                    .partial_cmp(&(b.io_read + b.io_write))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .filter(|p| p.io_read + p.io_write >= 1024.0 * 1024.0)
            .map(|p| (p.pid, p.name.clone(), p.io_read + p.io_write));
        if let Some(t) = short_avg(&|p| p.disk_temp) {
            let active = self.active.contains_key(&AnomalyKind::DiskTemp);
            let thr = settings.disk_temp_threshold;
            let on = short.len() >= 3 && t >= thr;
            let keep = active && t >= thr - 3.0;
            let level = if t >= thr + 8.0 { "critical" } else { "high" };
            let disk_name = self.hot_disk.clone().unwrap_or_else(|| "磁盘".into());
            let activity = short_avg(&|p| p.disk_active)
                .map(|a| format!("，磁盘活动时间 {:.0}%", a))
                .unwrap_or_default();
            let io_text = io_top
                .as_ref()
                .map(|(_, n, r)| format!("，读写最多的进程：{}（{}/s）", n, format_bytes(*r)))
                .unwrap_or_else(|| "，当前没有明显的高读写进程".into());
            detections.push((
                AnomalyKind::DiskTemp,
                on || keep,
                format!("{} 温度 {:.0}°C，超过告警阈值 {:.0}°C{}{}", disk_name, t, thr, activity, io_text),
                Some(round1(t)),
                Some(thr),
                level.into(),
            ));
        }

        // 风扇
        if let Some(rpm) = short_avg(&|p| p.fan) {
            // 维护低负载基线
            if sys_cpu_avg < 30.0 && sys_gpu_avg < 30.0 {
                self.fan_baseline = Some(match self.fan_baseline {
                    Some(b) => b * 0.98 + rpm * 0.02,
                    None => rpm,
                });
            }
            let thr = settings.fan_rpm_threshold;
            let rise = self
                .fan_baseline
                .map_or(false, |b| b > 300.0 && rpm > b * 1.6 && rpm - b > 800.0);
            let fan_high = rpm >= thr || rise;
            let active = self.active.contains_key(&AnomalyKind::FanHigh);
            let keep = active && (rpm >= thr * 0.92 || rise);
            let base_text = self
                .fan_baseline
                .map(|b| format!("（低负载基线约 {:.0} RPM）", b))
                .unwrap_or_default();
            detections.push((
                AnomalyKind::FanHigh,
                (short.len() >= 3 && fan_high) || keep,
                format!("风扇转速 {:.0} RPM{}，正在分析温度及负载变化{}", rpm, base_text, top_text),
                Some(round1(rpm)),
                Some(thr),
                "notice".into(),
            ));
            let active2 = self.active.contains_key(&AnomalyKind::FanHighLowLoad);
            let low_load = covered && sys_cpu_avg < 30.0 && sys_gpu_avg < 30.0;
            detections.push((
                AnomalyKind::FanHighLowLoad,
                (fan_high && low_load && short.len() >= 3) || (active2 && fan_high && sys_cpu_avg < 40.0),
                format!(
                    "风扇转速 {:.0} RPM，但 CPU 负载仅 {:.0}%、GPU 负载 {:.0}%，建议检查其他硬件或散热原因",
                    rpm, sys_cpu_avg, sys_gpu_avg
                ),
                Some(round1(rpm)),
                Some(thr),
                "notice".into(),
            ));
        }

        let mut new_anomalies = vec![];
        let mut seen: HashSet<AnomalyKind> = HashSet::new();
        for (kind, on, desc, value, thr, level) in detections {
            seen.insert(kind);
            let related = match kind {
                AnomalyKind::CpuHigh | AnomalyKind::CpuTemp | AnomalyKind::FanHigh => top.as_ref().map(|(p, n, _)| (*p, n.clone())),
                AnomalyKind::DiskTemp => io_top.as_ref().map(|(p, n, _)| (*p, n.clone())),
                _ => None,
            };
            if on {
                if let Some(a) = self.active.get_mut(&kind) {
                    a.value = value;
                    a.description = desc;
                    if level_rank(&level) > level_rank(&a.level) {
                        a.level = level;
                    }
                    if a.related.is_none() {
                        a.related = related;
                    }
                } else {
                    let db_id = db
                        .map(|d| {
                            d.insert_event(
                                now,
                                &level,
                                kind.title(),
                                &desc,
                                related.as_ref().map(|r| r.0),
                                related.as_ref().map(|r| r.1.as_str()),
                                "active",
                            )
                        })
                        .unwrap_or(0);
                    let a = ActiveAnomaly {
                        kind,
                        db_id,
                        since: now,
                        level,
                        value,
                        threshold: thr,
                        description: desc,
                        ignored: false,
                        related,
                    };
                    self.markers.push_back(Marker {
                        ts: now,
                        title: kind.title().into(),
                        level: a.level.clone(),
                    });
                    new_anomalies.push(Self::view(&a, now));
                    self.active.insert(kind, a);
                }
            } else if let Some(a) = self.active.remove(&kind) {
                if let Some(db) = db {
                    let status = if a.ignored { "ignored" } else { "resolved" };
                    db.update_event(a.db_id, status, Some(now), Some(&a.description), Some(&a.level));
                }
            }
        }
        // 数据源消失的异常（例如传感器断开）同样结束
        let gone: Vec<AnomalyKind> = self.active.keys().filter(|k| !seen.contains(k)).copied().collect();
        for k in gone {
            if let Some(a) = self.active.remove(&k) {
                if let Some(db) = db {
                    db.update_event(a.db_id, "resolved", Some(now), None, None);
                }
            }
        }
        while self.markers.front().map_or(false, |m| now - m.ts > SYS_KEEP_MS * 2) {
            self.markers.pop_front();
        }

        EngineOutput { new_anomalies }
    }

    fn top_suspect(&self, procs: &[ProcessInfo], settings: &Settings) -> Option<(u32, String, f64)> {
        procs
            .iter()
            .filter(|p| !settings.is_whitelisted(&p.name) && !settings.is_ignored(&p.name))
            .filter(|p| p.score >= 40.0 || p.cpu >= 15.0)
            .max_by(|a, b| {
                (a.score, a.cpu)
                    .partial_cmp(&(b.score, b.cpu))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|p| {
                let avg = self
                    .procs
                    .get(&(p.pid, p.create_time))
                    .map(|w| w.breakdown.avg_cpu)
                    .filter(|v| *v > 0.0)
                    .unwrap_or(p.cpu);
                (p.pid, p.name.clone(), avg)
            })
    }

    fn view(a: &ActiveAnomaly, now: i64) -> AnomalyView {
        AnomalyView {
            kind: a.kind,
            code: a.kind.code().into(),
            title: a.kind.title().into(),
            level: a.level.clone(),
            since: a.since,
            duration_secs: (now - a.since) / 1000,
            value: a.value,
            threshold: a.threshold,
            description: a.description.clone(),
            ignored: a.ignored,
            related_pid: a.related.as_ref().map(|r| r.0),
            related_name: a.related.as_ref().map(|r| r.1.clone()),
        }
    }

    pub fn active_views(&self, now: i64) -> Vec<AnomalyView> {
        let mut v: Vec<AnomalyView> = self.active.values().map(|a| Self::view(a, now)).collect();
        v.sort_by(|a, b| level_rank(&b.level).cmp(&level_rank(&a.level)).then(a.since.cmp(&b.since)));
        v
    }

    /// 当前整体异常等级（忽略被用户忽略的异常）
    pub fn overall_level(&self) -> String {
        self.active
            .values()
            .filter(|a| !a.ignored)
            .map(|a| a.level.clone())
            .max_by_key(|l| level_rank(l))
            .unwrap_or_else(|| "normal".into())
    }

    /// 忽略当前全部异常提示（异常结束前不再提示）
    pub fn ignore_all(&mut self, db: Option<&Db>) -> usize {
        let mut n = 0;
        for a in self.active.values_mut() {
            if !a.ignored {
                a.ignored = true;
                n += 1;
                if let Some(db) = db {
                    db.update_event(a.db_id, "ignored", None, None, None);
                }
            }
        }
        n
    }

    pub fn ignore(&mut self, code: &str, db: Option<&Db>) {
        for a in self.active.values_mut() {
            if a.kind.code() == code {
                a.ignored = true;
                if let Some(db) = db {
                    db.update_event(a.db_id, "ignored", None, None, None);
                }
            }
        }
    }

    pub fn markers(&self) -> Vec<Marker> {
        self.markers.iter().cloned().collect()
    }

    pub fn sys_points(&self) -> Vec<SysPoint> {
        self.sys.iter().copied().collect()
    }

    /// 进程 CPU / GPU 历史（详情窗口曲线）
    pub fn process_history(&self, key: ProcKey) -> Vec<(i64, f64, f64, u64)> {
        self.procs
            .get(&key)
            .map(|w| w.samples.iter().map(|s| (s.ts, s.cpu as f64, s.gpu as f64, s.mem)).collect())
            .unwrap_or_default()
    }

    pub fn process_breakdown(&self, key: ProcKey) -> Option<(f64, ScoreBreakdown)> {
        self.procs.get(&key).map(|w| (w.score, w.breakdown.clone()))
    }

    pub fn process_name(&self, key: ProcKey) -> Option<String> {
        self.procs.get(&key).map(|w| w.name.clone())
    }

    // -----------------------------------------------------------------------
    // 诊断报告
    // -----------------------------------------------------------------------

    pub fn report(
        &self,
        now: i64,
        procs: &[ProcessInfo],
        settings: &Settings,
        sensor_notes: Vec<String>,
        total_memory: u64,
        mem_usage: f64,
    ) -> DiagnosisReport {
        let anomalies = self.active_views(now);
        let hardware = self.sys.back().copied().unwrap_or_default();
        let window = Self::window_ms(settings) / 1000;

        // ---------- 主要排查对象 ----------
        let mut cands: Vec<&ProcessInfo> = procs
            .iter()
            .filter(|p| !settings.is_ignored(&p.name))
            .filter(|p| p.score > 0.0)
            .collect();
        cands.sort_by(|a, b| {
            let wa = settings.is_whitelisted(&a.name);
            let wb = settings.is_whitelisted(&b.name);
            // 白名单进程排在后面，但不隐藏
            wa.cmp(&wb)
                .then(b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal))
        });
        let mut suspects: Vec<Suspect> = vec![];
        let min_score = if anomalies.is_empty() { 40.0 } else { 20.0 };
        for p in cands.iter().filter(|p| p.score >= min_score).take(6) {
            suspects.push(self.make_suspect(p, settings, total_memory));
        }

        // ---------- 诊断等级与标题 ----------
        let level = self.overall_level();
        let max_score = suspects.iter().filter(|s| !s.whitelisted).map(|s| s.score).fold(0.0, f64::max);
        let level = if level == "normal" && max_score >= 60.0 {
            "notice".to_string()
        } else {
            level
        };
        let title = if let Some(a) = anomalies.iter().find(|a| !a.ignored) {
            match a.kind {
                AnomalyKind::CpuHigh | AnomalyKind::GpuHigh => "检测到电脑存在持续高负载".to_string(),
                AnomalyKind::CpuTemp | AnomalyKind::GpuTemp => "检测到硬件温度过高".to_string(),
                AnomalyKind::DiskTemp => "检测到磁盘温度过高".to_string(),
                AnomalyKind::FanHigh => "检测到风扇转速异常升高".to_string(),
                AnomalyKind::FanHighLowLoad | AnomalyKind::TempHighLowLoad => "负载不高但散热异常".to_string(),
            }
        } else if max_score >= 60.0 {
            "发现高负载进程，但尚未触发系统级异常".to_string()
        } else {
            "电脑运行状态正常".to_string()
        };
        let duration_secs = anomalies.iter().map(|a| a.duration_secs).max();

        // ---------- 诊断理由 ----------
        let mut reasons: Vec<String> = anomalies.iter().map(|a| a.description.clone()).collect();
        if reasons.is_empty() {
            reasons.push(format!(
                "最近 {} 秒 CPU 平均 {:.0}%{}，均未超过告警阈值",
                window,
                self.sys_avg(window * 1000, now, |p| Some(p.cpu)).unwrap_or(0.0),
                self.sys_avg(window * 1000, now, |p| p.gpu)
                    .map(|g| format!("、GPU 平均 {:.0}%", g))
                    .unwrap_or_default()
            ));
        }
        if let Some(s) = suspects.iter().find(|s| !s.whitelisted) {
            reasons.push(format!(
                "{} 的异常关联分数最高（{:.0} 分，{}），{}",
                s.name,
                s.score,
                s.grade_text,
                s.reasons.first().cloned().unwrap_or_default()
            ));
        }
        reasons.push("提示：关联分数是优先排查顺序，并非风扇狂转的真实因果贡献率；CPU 占用最高的进程不一定是唯一原因。".into());

        // ---------- 数据说明 ----------
        let mut data_notes = sensor_notes;
        if hardware.disk_temp.is_none() {
            data_notes.push("磁盘温度暂不支持读取（需要 LibreHardwareMonitor 识别到存储设备，部分硬盘或 RAID/USB 硬盘不提供温度）".into());
        }
        if !self.gpu_available {
            data_notes.push("当前系统不支持 GPU 性能计数器，GPU 使用率不可用，评分中 GPU 维度已移除".into());
        }
        if let Some(s) = suspects.first() {
            match s.breakdown.power_source.as_str() {
                "temperature" => data_notes.push("功耗传感器不可用，功耗关联维度改用温度变化估算（权重减半）".into()),
                "unavailable" => data_notes.push("功耗与温度数据均不可用，功耗关联维度已标记为不可用".into()),
                _ => {}
            }
        }

        // ---------- 优化建议 ----------
        let suggestions = self.suggestions(&anomalies, &suspects, &hardware, mem_usage);

        DiagnosisReport {
            generated_at: now,
            level_text: level_text(&level).into(),
            level,
            title,
            duration_secs,
            anomalies,
            suspects,
            hardware,
            reasons,
            suggestions,
            data_notes,
            window_secs: window,
        }
    }

    fn sys_avg(&self, window_ms: i64, now: i64, f: impl Fn(&SysPoint) -> Option<f64>) -> Option<f64> {
        avg_opt(self.sys.iter().filter(|p| now - p.ts <= window_ms).map(f))
    }

    pub fn make_suspect(&self, p: &ProcessInfo, settings: &Settings, total_memory: u64) -> Suspect {
        let b = self
            .procs
            .get(&(p.pid, p.create_time))
            .map(|w| w.breakdown.clone())
            .unwrap_or_default();
        let (grade, grade_text) = grade_of(p.score);
        let correlation = if p.score >= 70.0 {
            "高"
        } else if p.score >= 45.0 {
            "中"
        } else {
            "低"
        };
        let mut reasons = vec![];
        if b.avg_cpu >= 5.0 {
            reasons.push(format!("最近 {} 秒平均 CPU 占用 {:.1}%（整机）", b.window_secs, b.avg_cpu));
        }
        if b.gpu_available && b.avg_gpu >= 5.0 {
            reasons.push(format!("最近 {} 秒平均 GPU 占用 {:.1}%", b.window_secs, b.avg_gpu));
        }
        if b.sustained_secs >= 30 {
            reasons.push(format!("已持续高负载 {} 秒", b.sustained_secs));
        }
        if b.power >= 50.0 {
            reasons.push(match b.power_source.as_str() {
                "power" => "负载变化与 CPU/GPU 功耗变化高度相关".to_string(),
                _ => "负载变化与硬件温度变化高度相关".to_string(),
            });
        }
        if b.mem_growth > 200 * 1024 * 1024 {
            reasons.push(format!("内存在窗口期内增长 {}", format_bytes(b.mem_growth as f64)));
        } else if p.memory as f64 > total_memory as f64 * 0.15 {
            reasons.push(format!("内存占用较高（{}），但内存高不代表一定导致风扇高速旋转", format_bytes(p.memory as f64)));
        }
        if reasons.is_empty() {
            reasons.push("资源占用处于中等水平".into());
        }
        Suspect {
            pid: p.pid,
            create_time: p.create_time,
            name: p.name.clone(),
            path: p.path.clone(),
            cpu_avg: b.avg_cpu,
            gpu_avg: if b.gpu_available { Some(b.avg_gpu) } else { None },
            memory: p.memory,
            duration_secs: b.sustained_secs,
            score: p.score,
            grade: grade.into(),
            grade_text: grade_text.into(),
            correlation: correlation.into(),
            reasons,
            breakdown: b,
            protected: p.kind == ProcKind::Critical,
            whitelisted: settings.is_whitelisted(&p.name),
            kind: p.kind,
        }
    }

    fn suggestions(&self, anomalies: &[AnomalyView], suspects: &[Suspect], hw: &SysPoint, mem_usage: f64) -> Vec<Suggestion> {
        let mut out: Vec<Suggestion> = vec![];
        let push = |out: &mut Vec<Suggestion>, title: &str, detail: String, s: Option<&Suspect>| {
            if out.iter().any(|x| x.title == title) {
                return;
            }
            out.push(Suggestion {
                title: title.into(),
                detail,
                related_pid: s.map(|s| s.pid),
                related_name: s.map(|s| s.name.clone()),
            });
        };
        for s in suspects.iter().filter(|s| !s.whitelisted && s.score >= 40.0).take(4) {
            let n = s.name.to_lowercase();
            let is = |list: &[&str]| list.iter().any(|x| n == *x || n.starts_with(x));
            if is(&["chrome.exe", "msedge.exe", "firefox.exe", "opera.exe", "brave.exe", "vivaldi.exe", "360se.exe", "360chrome.exe", "qqbrowser.exe", "sogouexplorer.exe", "msedgewebview2.exe"]) {
                push(&mut out, 
                    "浏览器 CPU 占用过高",
                    format!("{} 占用较高：检查浏览器标签页及扩展。可通过浏览器自带的任务管理器（Shift+Esc）定位具体标签页，关闭视频、动画或挖矿脚本页面，禁用不必要的扩展。", s.name),
                    Some(s),
                );
            } else if is(&["node.exe", "bun.exe", "deno.exe", "npm", "yarn", "pnpm"]) {
                push(&mut out, 
                    "Node.js 占用过高",
                    format!("{} 占用较高：检查开发服务器、构建任务（如 Vite/Webpack 监听、测试 watch、TypeScript 编译）是否在后台持续运行。", s.name),
                    Some(s),
                );
            } else if is(&["python.exe", "pythonw.exe", "py.exe", "python3.exe", "conda.exe", "jupyter"]) {
                push(&mut out, 
                    "Python 占用过高",
                    format!("{} 占用较高：检查运行中的脚本、Jupyter 内核或训练任务是否已完成或陷入死循环。", s.name),
                    Some(s),
                );
            } else if is(&["java.exe", "javaw.exe", "idea64.exe", "studio64.exe", "pycharm64.exe", "webstorm64.exe", "rider64.exe", "clion64.exe", "goland64.exe"]) {
                push(&mut out, 
                    "Java / JetBrains IDE 占用过高",
                    format!("{} 占用较高：IDE 正在建立索引或 Gradle/Maven 守护进程在后台构建，可等待完成或在 IDE 中暂停索引。", s.name),
                    Some(s),
                );
            } else if is(&["code.exe", "devenv.exe", "cursor.exe"]) {
                push(&mut out, 
                    "代码编辑器占用过高",
                    format!("{} 占用较高：检查扩展（语言服务器、ESLint、文件监视）或大型工作区索引。", s.name),
                    Some(s),
                );
            } else if is(&["vmmem", "vmmemws", "docker", "com.docker", "wsl", "vmwp.exe", "vmware-vmx.exe", "virtualboxvm.exe"]) {
                push(&mut out, 
                    "虚拟机/容器占用过高",
                    format!("{} 占用较高：检查 WSL、Docker 容器或虚拟机中运行的任务，可通过 wsl --shutdown 或停止容器释放资源。", s.name),
                    Some(s),
                );
            } else if is(&["msmpeng.exe", "tiworker.exe", "trustedinstaller.exe", "searchindexer.exe", "compattelrunner.exe", "mousocoreworker.exe", "wuauclt.exe", "musnotification.exe"]) {
                push(&mut out, 
                    "系统维护任务占用较高",
                    format!("{} 属于系统维护任务（安全扫描、Windows 更新或索引），通常会自行结束，建议等待其完成，或调整其计划运行时间。不建议结束。", s.name),
                    Some(s),
                );
            } else if is(&["obs64.exe", "ffmpeg.exe", "handbrake.exe", "adobe premiere pro.exe", "afterfx.exe", "blender.exe", "resolve.exe"]) {
                push(&mut out, 
                    "渲染/编码任务",
                    format!("{} 正在进行渲染或编码，属于预期的高负载，任务完成后负载会下降。", s.name),
                    Some(s),
                );
            } else if s.protected {
                push(&mut out, 
                    "系统进程负载较高",
                    format!("{} 是受保护的系统进程，不能结束。可能由驱动、系统服务或外设引起，建议检查 Windows 更新、驱动程序或事件查看器。", s.name),
                    Some(s),
                );
            } else {
                push(&mut out, 
                    &format!("检查 {}", s.name),
                    format!("检查 {} 对应的程序或后台任务是否正在执行预期之外的工作；若确认无用，可在进程管理中结束它。", s.name),
                    Some(s),
                );
            }
        }
        let has = |k: AnomalyKind| anomalies.iter().any(|a| a.kind == k);
        if has(AnomalyKind::GpuHigh) {
            let s = suspects.iter().find(|s| s.gpu_avg.unwrap_or(0.0) >= 10.0);
            push(&mut out, "GPU 占用过高", "检查游戏、视频渲染、直播推流及浏览器 / 应用的 GPU 加速任务。游戏退出后 GPU 仍高负载时，检查是否有残留进程。".into(), s);
        }
        if has(AnomalyKind::TempHighLowLoad) || has(AnomalyKind::FanHighLowLoad) {
            push(&mut out, 
                "CPU/GPU 负载正常但温度或风扇异常",
                "检查散热系统：清理灰尘、确认风道通畅、检查硅脂老化与散热器安装、降低环境温度；笔记本请避免放在软物表面。也可能是显卡、电源或硬盘等其他硬件发热导致。".into(),
                None,
            );
        }
        if (has(AnomalyKind::CpuTemp) || has(AnomalyKind::GpuTemp)) && !has(AnomalyKind::CpuHigh) && !has(AnomalyKind::GpuHigh) {
            push(&mut out, 
                "高温但负载未持续超标",
                "温度已超过阈值，但负载并未持续超标，建议检查散热器、风扇是否正常运转以及机箱风道。".into(),
                None,
            );
        }
        if let Some(a) = anomalies.iter().find(|a| a.kind == AnomalyKind::DiskTemp) {
            let busy = hw.disk_active.map_or(false, |v| v >= 50.0);
            if busy || a.related_pid.is_some() {
                out.push(Suggestion {
                    title: "磁盘持续高负载导致发热".into(),
                    detail: format!(
                        "磁盘正在大量读写{}。检查下载、杀毒扫描、文件索引、虚拟机、数据库或备份同步等任务，必要时暂停或改到空闲时间运行。",
                        a.related_name.as_ref().map(|n| format!("（读写最多的进程：{}）", n)).unwrap_or_default()
                    ),
                    related_pid: a.related_pid,
                    related_name: a.related_name.clone(),
                });
            }
            push(&mut out,
                "改善磁盘散热",
                if busy {
                    "为 NVMe 固态硬盘加装散热片或使用主板自带散热马甲，确认机箱前进风与硬盘位风道通畅；长时间高温会触发固态降速并缩短寿命。".into()
                } else {
                    "磁盘负载不高但温度偏高，可能是散热不良或机箱内其他硬件（显卡等）烘烤所致。建议检查硬盘位风道、加装散热片，并用 SMART 工具检查硬盘健康状况。".into()
                },
                None,
            );
        }
        if hw.fan.is_none() && hw.cpu_temp.is_none() {
            push(&mut out, 
                "风扇高速但传感器数据不可用",
                "当前硬件未提供风扇转速/温度数据。若风扇持续高速，建议检查 BIOS 风扇策略、厂商控制软件（如笔记本性能模式）以及硬件风扇策略；以管理员身份运行 FanGuard 可读取更多传感器。".into(),
                None,
            );
        }
        if mem_usage >= 90.0 {
            push(&mut out, 
                "内存使用率过高",
                format!("内存使用率 {:.0}%，系统会频繁使用页面文件，导致磁盘与 CPU 负载上升。建议关闭不需要的程序。", mem_usage),
                None,
            );
        }
        if out.is_empty() {
            push(&mut out, 
                "暂无需要处理的问题",
                "当前没有检测到持续高负载或高温。如果风扇仍然较响，可能是厂商风扇策略或短时负载波动，FanGuard 会持续监控。".into(),
                None,
            );
        }
        out
    }
}
