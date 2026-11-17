//! Windows 性能计数器（PDH）采集
//!
//! - GPU Engine：每个进程、每个 GPU 引擎的使用率（与任务管理器同源）
//! - GPU Process Memory：每个进程的专用显存
//! - GPU Adapter Memory：每块显卡的显存占用
//! - PhysicalDisk：磁盘读写速度、活动时间
//! - Processor Information：处理器性能百分比（用于估算实时频率）
//! - Paging File：页面文件使用率

use crate::util::wide;
use std::collections::HashMap;

type PdhHandle = isize;

#[repr(C)]
#[derive(Clone, Copy)]
struct PdhFmtValue {
    cstatus: u32,
    // union { long, double, longlong, ptr } —— 8 字节对齐
    value: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct PdhFmtItem {
    name: *const u16,
    value: PdhFmtValue,
}

#[link(name = "pdh")]
extern "system" {
    fn PdhOpenQueryW(source: *const u16, user: usize, query: *mut PdhHandle) -> u32;
    fn PdhAddEnglishCounterW(query: PdhHandle, path: *const u16, user: usize, counter: *mut PdhHandle) -> u32;
    fn PdhCollectQueryData(query: PdhHandle) -> u32;
    fn PdhGetFormattedCounterArrayW(
        counter: PdhHandle,
        format: u32,
        buffer_size: *mut u32,
        item_count: *mut u32,
        items: *mut PdhFmtItem,
    ) -> u32;
    fn PdhGetFormattedCounterValue(
        counter: PdhHandle,
        format: u32,
        ctype: *mut u32,
        value: *mut PdhFmtValue,
    ) -> u32;
    fn PdhCloseQuery(query: PdhHandle) -> u32;
}

const PDH_FMT_DOUBLE: u32 = 0x0000_0200;
const PDH_FMT_NOCAP100: u32 = 0x0000_8000;
const PDH_MORE_DATA: u32 = 0x8000_07D2;
const PDH_CSTATUS_VALID_DATA: u32 = 0;
const PDH_CSTATUS_NEW_DATA: u32 = 1;

struct Counter {
    handle: PdhHandle,
}

pub struct PdhQuery {
    query: PdhHandle,
    counters: HashMap<&'static str, Counter>,
}

unsafe impl Send for PdhQuery {}

impl PdhQuery {
    fn new() -> Option<Self> {
        let mut q: PdhHandle = 0;
        let r = unsafe { PdhOpenQueryW(std::ptr::null(), 0, &mut q) };
        if r != 0 {
            return None;
        }
        Some(Self {
            query: q,
            counters: HashMap::new(),
        })
    }

    fn add(&mut self, key: &'static str, path: &str) -> bool {
        let w = wide(path);
        let mut c: PdhHandle = 0;
        let r = unsafe { PdhAddEnglishCounterW(self.query, w.as_ptr(), 0, &mut c) };
        if r == 0 {
            self.counters.insert(key, Counter { handle: c });
            true
        } else {
            false
        }
    }

    fn has(&self, key: &str) -> bool {
        self.counters.contains_key(key)
    }

    fn collect(&self) -> bool {
        unsafe { PdhCollectQueryData(self.query) == 0 }
    }

    fn single(&self, key: &str) -> Option<f64> {
        let c = self.counters.get(key)?;
        let mut v = PdhFmtValue { cstatus: 0, value: 0.0 };
        let r = unsafe {
            PdhGetFormattedCounterValue(c.handle, PDH_FMT_DOUBLE | PDH_FMT_NOCAP100, std::ptr::null_mut(), &mut v)
        };
        if r == 0 && (v.cstatus == PDH_CSTATUS_VALID_DATA || v.cstatus == PDH_CSTATUS_NEW_DATA) {
            Some(v.value)
        } else {
            None
        }
    }

    fn array(&self, key: &str) -> Option<Vec<(String, f64)>> {
        let c = self.counters.get(key)?;
        let mut size: u32 = 0;
        let mut count: u32 = 0;
        let fmt = PDH_FMT_DOUBLE | PDH_FMT_NOCAP100;
        let r = unsafe { PdhGetFormattedCounterArrayW(c.handle, fmt, &mut size, &mut count, std::ptr::null_mut()) };
        if r != PDH_MORE_DATA || size == 0 {
            return if r == 0 { Some(vec![]) } else { None };
        }
        // 缓冲区大小以字节计，包含条目数组以及其后的实例名字符串；使用 u64 保证对齐
        let mut raw: Vec<u64> = vec![0; size as usize / 8 + 1];
        let r = unsafe {
            PdhGetFormattedCounterArrayW(c.handle, fmt, &mut size, &mut count, raw.as_mut_ptr() as *mut PdhFmtItem)
        };
        if r != 0 {
            return None;
        }
        let buf: &[PdhFmtItem] =
            unsafe { std::slice::from_raw_parts(raw.as_ptr() as *const PdhFmtItem, count as usize) };
        let mut out = Vec::with_capacity(count as usize);
        for item in buf {
            if item.value.cstatus != PDH_CSTATUS_VALID_DATA && item.value.cstatus != PDH_CSTATUS_NEW_DATA {
                continue;
            }
            let name = unsafe {
                if item.name.is_null() {
                    String::new()
                } else {
                    let mut len = 0usize;
                    while *item.name.add(len) != 0 {
                        len += 1;
                    }
                    String::from_utf16_lossy(std::slice::from_raw_parts(item.name, len))
                }
            };
            out.push((name, item.value.value));
        }
        Some(out)
    }
}

impl Drop for PdhQuery {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.query) };
    }
}

#[derive(Debug, Clone, Default)]
pub struct EngineUsage {
    pub luid: String,
    pub engine: String,
    pub usage: f64,
}

#[derive(Debug, Clone, Default)]
pub struct AdapterUsage {
    /// 最忙引擎的使用率（与任务管理器一致）
    pub usage: f64,
    pub engines: Vec<(String, f64)>,
    pub dedicated_used: Option<u64>,
    pub shared_used: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct PdhSample {
    pub gpu_available: bool,
    pub proc_gpu: HashMap<u32, f64>,
    pub proc_gpu_mem: HashMap<u32, u64>,
    pub proc_engines: HashMap<u32, Vec<EngineUsage>>,
    pub adapters: HashMap<String, AdapterUsage>,
    pub disk_read: Option<f64>,
    pub disk_write: Option<f64>,
    pub disk_active: Option<f64>,
    pub cpu_perf: Option<f64>,
    pub pagefile_usage: Option<f64>,
}

/// 解析实例名中的字段，例如 pid_1234_luid_0x00000000_0x0000C5F0_phys_0_eng_0_engtype_3D
fn parse_field<'a>(inst: &'a str, key: &str) -> Option<&'a str> {
    let idx = inst.find(key)?;
    let rest = &inst[idx + key.len()..];
    Some(rest)
}

fn parse_pid(inst: &str) -> Option<u32> {
    let rest = parse_field(inst, "pid_")?;
    let end = rest.find('_').unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn parse_luid(inst: &str) -> Option<String> {
    let rest = parse_field(inst, "luid_")?;
    // 0x00000000_0x0000C5F0
    if rest.len() >= 21 {
        Some(rest[..21].to_uppercase().replace("0X", "0x"))
    } else {
        None
    }
}

fn parse_engtype(inst: &str) -> String {
    parse_field(inst, "engtype_")
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Other".into())
}

pub struct PdhCollector {
    q: Option<PdhQuery>,
    primed: bool,
}

impl PdhCollector {
    pub fn new() -> Self {
        let q = PdhQuery::new().map(|mut q| {
            q.add("gpu_engine", "\\GPU Engine(*)\\Utilization Percentage");
            q.add("gpu_proc_mem", "\\GPU Process Memory(*)\\Dedicated Usage");
            q.add("gpu_adapter_ded", "\\GPU Adapter Memory(*)\\Dedicated Usage");
            q.add("gpu_adapter_shr", "\\GPU Adapter Memory(*)\\Shared Usage");
            q.add("disk_read", "\\PhysicalDisk(_Total)\\Disk Read Bytes/sec");
            q.add("disk_write", "\\PhysicalDisk(_Total)\\Disk Write Bytes/sec");
            q.add("disk_idle", "\\PhysicalDisk(_Total)\\% Idle Time");
            q.add("cpu_perf", "\\Processor Information(_Total)\\% Processor Performance");
            q.add("pagefile", "\\Paging File(_Total)\\% Usage");
            q
        });
        let c = Self { q, primed: false };
        if let Some(q) = &c.q {
            q.collect();
        }
        c
    }

    pub fn sample(&mut self) -> PdhSample {
        let mut s = PdhSample::default();
        let q = match &self.q {
            Some(q) => q,
            None => return s,
        };
        if !q.collect() && !self.primed {
            return s;
        }
        self.primed = true;

        // ----- GPU 引擎 -----
        if let Some(items) = q.array("gpu_engine") {
            s.gpu_available = q.has("gpu_engine");
            // (pid, luid, engtype) -> sum
            let mut per_proc: HashMap<(u32, String, String), f64> = HashMap::new();
            // (luid, engtype) -> sum
            let mut per_adapter: HashMap<(String, String), f64> = HashMap::new();
            for (inst, v) in items {
                if v <= 0.0 {
                    // 仍需要记录适配器以便显示 0%
                    if let Some(luid) = parse_luid(&inst) {
                        per_adapter.entry((luid, parse_engtype(&inst))).or_insert(0.0);
                    }
                    continue;
                }
                let (pid, luid) = match (parse_pid(&inst), parse_luid(&inst)) {
                    (Some(p), Some(l)) => (p, l),
                    _ => continue,
                };
                let eng = parse_engtype(&inst);
                *per_proc.entry((pid, luid.clone(), eng.clone())).or_insert(0.0) += v;
                *per_adapter.entry((luid, eng)).or_insert(0.0) += v;
            }
            for ((pid, luid, eng), v) in per_proc {
                let v = v.min(100.0);
                let e = s.proc_gpu.entry(pid).or_insert(0.0);
                if v > *e {
                    *e = v;
                }
                s.proc_engines.entry(pid).or_default().push(EngineUsage {
                    luid,
                    engine: eng,
                    usage: crate::util::round1(v),
                });
            }
            for ((luid, eng), v) in per_adapter {
                let v = v.min(100.0);
                let a = s.adapters.entry(luid).or_default();
                a.engines.push((eng, crate::util::round1(v)));
                if v > a.usage {
                    a.usage = v;
                }
            }
            for a in s.adapters.values_mut() {
                a.engines.sort_by(|x, y| y.1.partial_cmp(&x.1).unwrap_or(std::cmp::Ordering::Equal));
            }
        }
        if let Some(items) = q.array("gpu_proc_mem") {
            for (inst, v) in items {
                if let Some(pid) = parse_pid(&inst) {
                    *s.proc_gpu_mem.entry(pid).or_insert(0) += v.max(0.0) as u64;
                }
            }
        }
        if let Some(items) = q.array("gpu_adapter_ded") {
            for (inst, v) in items {
                if let Some(luid) = parse_luid(&inst) {
                    let a = s.adapters.entry(luid).or_default();
                    *a.dedicated_used.get_or_insert(0) += v.max(0.0) as u64;
                }
            }
        }
        if let Some(items) = q.array("gpu_adapter_shr") {
            for (inst, v) in items {
                if let Some(luid) = parse_luid(&inst) {
                    let a = s.adapters.entry(luid).or_default();
                    *a.shared_used.get_or_insert(0) += v.max(0.0) as u64;
                }
            }
        }

        // ----- 磁盘 / CPU / 页面文件 -----
        s.disk_read = q.single("disk_read");
        s.disk_write = q.single("disk_write");
        s.disk_active = q.single("disk_idle").map(|idle| (100.0 - idle).clamp(0.0, 100.0));
        s.cpu_perf = q.single("cpu_perf");
        s.pagefile_usage = q.single("pagefile");
        s
    }
}
