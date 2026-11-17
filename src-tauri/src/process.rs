//! 进程采集模块
//!
//! 通过 NtQuerySystemInformation(SystemProcessInformation) 一次性获取全部进程及线程信息，
//! 相比逐个 OpenProcess 开销更低，并且对受保护进程同样可以读取 CPU 时间与内存。
//! CPU 使用率 = 进程 CPU 时间增量 / (墙钟时间增量 × 逻辑处理器数)，即"整机百分比"。

use crate::util::{filetime_to_unix_ms, from_wide, system_root};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::ffi::c_void;
use std::time::Instant;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};

#[repr(C)]
#[derive(Clone, Copy)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SystemProcessInformation {
    next_entry_offset: u32,
    number_of_threads: u32,
    working_set_private_size: i64,
    hard_fault_count: u32,
    number_of_threads_high_watermark: u32,
    cycle_time: u64,
    create_time: i64,
    user_time: i64,
    kernel_time: i64,
    image_name: UnicodeString,
    base_priority: i32,
    unique_process_id: usize,
    inherited_from_unique_process_id: usize,
    handle_count: u32,
    session_id: u32,
    unique_process_key: usize,
    peak_virtual_size: usize,
    virtual_size: usize,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_page_count: usize,
    read_operation_count: i64,
    write_operation_count: i64,
    other_operation_count: i64,
    read_transfer_count: i64,
    write_transfer_count: i64,
    other_transfer_count: i64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SystemThreadInformation {
    kernel_time: i64,
    user_time: i64,
    create_time: i64,
    wait_time: u32,
    start_address: *mut c_void,
    client_process: usize,
    client_thread: usize,
    priority: i32,
    base_priority: i32,
    context_switches: u32,
    thread_state: u32,
    wait_reason: u32,
}

#[link(name = "ntdll")]
extern "system" {
    fn NtQuerySystemInformation(
        class: u32,
        info: *mut c_void,
        length: u32,
        return_length: *mut u32,
    ) -> i32;
}

const SYSTEM_PROCESS_INFORMATION_CLASS: u32 = 5;
const SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION_CLASS: u32 = 8;
const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004u32 as i32;

/// 一次原始采样得到的进程数据
#[derive(Clone)]
pub struct RawProcess {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub create_time: i64,
    pub cpu_time: i64, // 100ns
    pub working_set: u64,
    pub private_ws: u64,
    pub private_bytes: u64,
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub threads: u32,
    pub handles: u32,
    pub session_id: u32,
    #[allow(dead_code)]
    pub base_priority: i32,
    pub thread_list: Vec<RawThread>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RawThread {
    pub tid: u32,
    pub cpu_time_ms: f64,
    pub kernel_time_ms: f64,
    pub user_time_ms: f64,
    pub start_time: i64,
    pub priority: i32,
    pub base_priority: i32,
    pub context_switches: u32,
    pub state: String,
    pub wait_reason: String,
    pub start_address: String,
}

fn query_buffer(class: u32, initial: usize) -> Option<Vec<u8>> {
    let mut size = initial;
    for _ in 0..8 {
        // 结构体统一使用 read_unaligned 读取，因此普通字节缓冲区即可
        let mut buf: Vec<u8> = vec![0; size];
        let mut ret: u32 = 0;
        let status = unsafe {
            NtQuerySystemInformation(class, buf.as_mut_ptr() as *mut c_void, buf.len() as u32, &mut ret)
        };
        if status == STATUS_INFO_LENGTH_MISMATCH {
            size = (ret as usize).max(size * 2) + 64 * 1024;
            continue;
        }
        if status < 0 {
            return None;
        }
        buf.truncate((ret as usize).min(buf.len()));
        return Some(buf);
    }
    None
}

/// 以对齐方式读取缓冲区中的结构体
unsafe fn read_at<T: Copy>(buf: &[u8], offset: usize) -> Option<T> {
    if offset + std::mem::size_of::<T>() > buf.len() {
        return None;
    }
    Some(std::ptr::read_unaligned(buf.as_ptr().add(offset) as *const T))
}

fn thread_state_name(s: u32) -> &'static str {
    match s {
        0 => "已初始化",
        1 => "就绪",
        2 => "运行中",
        3 => "待命",
        4 => "已终止",
        5 => "等待",
        6 => "转换",
        7 => "延迟就绪",
        8 => "门等待",
        9 => "等待换入",
        _ => "未知",
    }
}

fn wait_reason_name(r: u32) -> &'static str {
    match r {
        0 | 7 => "执行体",
        1 | 8 => "空闲页",
        2 | 9 => "页面读取",
        3 | 10 => "池分配",
        4 | 11 => "延迟执行",
        5 | 12 => "挂起",
        6 | 13 => "用户请求",
        14 => "事件对",
        15 => "队列",
        16 => "LPC 接收",
        17 => "LPC 应答",
        18 => "虚拟内存",
        19 => "页面写出",
        20 => "Rendezvous",
        31 => "Alert by ThreadId",
        _ => "其他",
    }
}

/// 查询全部进程（with_threads = true 时同时解析线程列表）
pub fn query_processes(with_threads: bool, only_pid: Option<u32>) -> Vec<RawProcess> {
    let buf = match query_buffer(SYSTEM_PROCESS_INFORMATION_CLASS, 512 * 1024) {
        Some(b) => b,
        None => return vec![],
    };
    let mut out = Vec::with_capacity(400);
    let mut offset = 0usize;
    let base = buf.as_ptr() as usize;
    loop {
        let info: SystemProcessInformation = match unsafe { read_at(&buf, offset) } {
            Some(i) => i,
            None => break,
        };
        let pid = info.unique_process_id as u32;
        if only_pid.map_or(true, |p| p == pid) {
            let name = if info.image_name.buffer.is_null() || info.image_name.length == 0 {
                if pid == 0 {
                    "System Idle Process".to_string()
                } else {
                    "System".to_string()
                }
            } else {
                // ImageName.Buffer 指向本缓冲区内部，按偏移读取以保证安全
                let ptr = info.image_name.buffer as usize;
                let len = (info.image_name.length / 2) as usize;
                if ptr >= base && ptr + len * 2 <= base + buf.len() {
                    let start = ptr - base;
                    let mut chars = Vec::with_capacity(len);
                    for i in 0..len {
                        let lo = buf[start + i * 2] as u16;
                        let hi = buf[start + i * 2 + 1] as u16;
                        chars.push(lo | (hi << 8));
                    }
                    String::from_utf16_lossy(&chars)
                } else {
                    format!("pid_{}", pid)
                }
            };

            let mut thread_list = Vec::new();
            if with_threads {
                let tbase = offset + std::mem::size_of::<SystemProcessInformation>();
                let tsize = std::mem::size_of::<SystemThreadInformation>();
                for i in 0..info.number_of_threads as usize {
                    let t: SystemThreadInformation = match unsafe { read_at(&buf, tbase + i * tsize) } {
                        Some(t) => t,
                        None => break,
                    };
                    thread_list.push(RawThread {
                        tid: t.client_thread as u32,
                        cpu_time_ms: (t.kernel_time + t.user_time) as f64 / 10_000.0,
                        kernel_time_ms: t.kernel_time as f64 / 10_000.0,
                        user_time_ms: t.user_time as f64 / 10_000.0,
                        start_time: filetime_to_unix_ms(t.create_time),
                        priority: t.priority,
                        base_priority: t.base_priority,
                        context_switches: t.context_switches,
                        state: thread_state_name(t.thread_state).to_string(),
                        wait_reason: if t.thread_state == 5 {
                            wait_reason_name(t.wait_reason).to_string()
                        } else {
                            String::new()
                        },
                        start_address: format!("0x{:X}", t.start_address as usize),
                    });
                }
            }

            out.push(RawProcess {
                pid,
                ppid: info.inherited_from_unique_process_id as u32,
                name,
                create_time: info.create_time,
                cpu_time: info.kernel_time + info.user_time,
                working_set: info.working_set_size as u64,
                private_ws: info.working_set_private_size.max(0) as u64,
                private_bytes: info.private_page_count as u64,
                read_bytes: info.read_transfer_count.max(0) as u64,
                write_bytes: info.write_transfer_count.max(0) as u64,
                threads: info.number_of_threads,
                handles: info.handle_count,
                session_id: info.session_id,
                base_priority: info.base_priority,
                thread_list,
            });
        }
        if info.next_entry_offset == 0 {
            break;
        }
        offset += info.next_entry_offset as usize;
    }
    out
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ProcessorPerformanceInfo {
    idle_time: i64,
    kernel_time: i64,
    user_time: i64,
    dpc_time: i64,
    interrupt_time: i64,
    interrupt_count: u32,
}

/// 每个逻辑处理器的 (idle, total) 累计时间
pub fn query_per_core_times(ncpu: usize) -> Vec<(i64, i64)> {
    let size = std::mem::size_of::<ProcessorPerformanceInfo>();
    let mut buf: Vec<ProcessorPerformanceInfo> = vec![ProcessorPerformanceInfo::default(); ncpu.max(1)];
    let mut ret = 0u32;
    let status = unsafe {
        NtQuerySystemInformation(
            SYSTEM_PROCESSOR_PERFORMANCE_INFORMATION_CLASS,
            buf.as_mut_ptr() as *mut c_void,
            (buf.len() * size) as u32,
            &mut ret,
        )
    };
    if status < 0 {
        return vec![];
    }
    let n = (ret as usize / size).min(buf.len());
    buf[..n]
        .iter()
        // KernelTime 包含 IdleTime
        .map(|p| (p.idle_time, p.kernel_time + p.user_time))
        .collect()
}

// ---------------------------------------------------------------------------
// 进程分类与保护
// ---------------------------------------------------------------------------

/// 关键 Windows 系统进程：禁止结束
const CRITICAL_NAMES: &[&str] = &[
    "system idle process",
    "system",
    "secure system",
    "registry",
    "memory compression",
    "smss.exe",
    "csrss.exe",
    "wininit.exe",
    "winlogon.exe",
    "services.exe",
    "lsass.exe",
    "lsaiso.exe",
    "svchost.exe",
    "dwm.exe",
    "fontdrvhost.exe",
    "msmpeng.exe",
    "nissrv.exe",
    "sgrmbroker.exe",
    "securityhealthservice.exe",
    "wudfhost.exe",
    "ntoskrnl.exe",
    "logonui.exe",
    "lsm.exe",
    "vmmem",
    "vmmemws",
];

/// 结束会造成桌面异常的系统进程：允许但显示强警告
const SENSITIVE_NAMES: &[&str] = &[
    "explorer.exe",
    "sihost.exe",
    "ctfmon.exe",
    "audiodg.exe",
    "spoolsv.exe",
    "taskhostw.exe",
    "runtimebroker.exe",
    "searchhost.exe",
    "startmenuexperiencehost.exe",
    "shellexperiencehost.exe",
    "textinputhost.exe",
    "conhost.exe",
    "dllhost.exe",
    "wmiprvse.exe",
    "searchindexer.exe",
];

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub enum ProcKind {
    /// 关键系统进程（受保护）
    Critical,
    /// 系统服务（Session 0）
    Service,
    /// Windows 系统组件（位于系统目录）
    System,
    /// 普通用户进程
    User,
}

pub fn classify(pid: u32, name: &str, session_id: u32, path: Option<&str>) -> ProcKind {
    let lname = name.to_lowercase();
    let sysroot = system_root().to_lowercase();
    let in_sysdir = path.map(|p| p.to_lowercase().starts_with(&sysroot));
    if pid <= 4 {
        return ProcKind::Critical;
    }
    if CRITICAL_NAMES.contains(&lname.as_str()) {
        // 同名但明确位于系统目录之外的进程不享受保护（可能是伪装进程）；
        // Defender 引擎位于 ProgramData，属于例外
        let defender = lname == "msmpeng.exe" || lname == "nissrv.exe";
        if in_sysdir != Some(false) || defender {
            return ProcKind::Critical;
        }
    }
    if session_id == 0 {
        return ProcKind::Service;
    }
    if in_sysdir == Some(true) {
        return ProcKind::System;
    }
    if in_sysdir.is_none() && SENSITIVE_NAMES.contains(&lname.as_str()) {
        return ProcKind::System;
    }
    ProcKind::User
}

pub fn is_sensitive(name: &str) -> bool {
    SENSITIVE_NAMES.contains(&name.to_lowercase().as_str())
}

// ---------------------------------------------------------------------------
// 进程路径
// ---------------------------------------------------------------------------

pub struct ProcessHandle(pub HANDLE);

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

pub fn open_process(pid: u32, access: u32) -> Option<ProcessHandle> {
    let h = unsafe { OpenProcess(access, 0, pid) };
    if h.is_null() {
        None
    } else {
        Some(ProcessHandle(h))
    }
}

pub fn query_image_path(pid: u32) -> Option<String> {
    let h = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(h.0, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) };
    if ok == 0 {
        return None;
    }
    Some(from_wide(&buf[..len as usize]))
}

// ---------------------------------------------------------------------------
// 进程采集器（计算增量）
// ---------------------------------------------------------------------------

/// 进程唯一标识：PID + 创建时间，防止 PID 重用导致数据错乱
pub type ProcKey = (u32, i64);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInfo {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    /// 创建时间（FILETIME 原始值，用于身份校验）
    #[serde(serialize_with = "crate::util::ser_i64_str")]
    pub create_time: i64,
    /// 启动时间（Unix 毫秒）
    pub start_time: i64,
    /// 整机 CPU 百分比
    pub cpu: f64,
    /// 内存（专用工作集，字节），与任务管理器"内存"列一致
    pub memory: u64,
    pub working_set: u64,
    pub private_bytes: u64,
    /// GPU 使用率（PDH 不可用时为 None）
    pub gpu: Option<f64>,
    /// 专用显存（字节）
    pub gpu_mem: Option<u64>,
    /// I/O 读取速率（字节/秒）
    pub io_read: f64,
    /// I/O 写入速率（字节/秒）
    pub io_write: f64,
    pub threads: u32,
    pub handles: u32,
    pub session_id: u32,
    pub path: Option<String>,
    pub kind: ProcKind,
    /// normal / abnormal / protected
    pub status: String,
    /// 异常关联分数 0~100
    pub score: f64,
    pub whitelisted: bool,
    pub ignored: bool,
}

struct PrevSample {
    cpu_time: i64,
    read: u64,
    write: u64,
}

pub struct ProcessCollector {
    prev: HashMap<ProcKey, PrevSample>,
    paths: HashMap<ProcKey, Option<String>>,
    last: Option<Instant>,
    pub ncpu: usize,
}

impl ProcessCollector {
    pub fn new(ncpu: usize) -> Self {
        Self {
            prev: HashMap::new(),
            paths: HashMap::new(),
            last: None,
            ncpu: ncpu.max(1),
        }
    }

    pub fn sample(&mut self) -> Vec<ProcessInfo> {
        let now = Instant::now();
        let raw = query_processes(false, None);
        let elapsed = self
            .last
            .map(|l| now.duration_since(l).as_secs_f64())
            .unwrap_or(0.0);
        self.last = Some(now);

        let mut next_prev = HashMap::with_capacity(raw.len());
        let mut alive: HashSet<ProcKey> = HashSet::with_capacity(raw.len());
        let mut out = Vec::with_capacity(raw.len());

        for p in raw {
            if p.pid == 0 {
                continue;
            }
            let key = (p.pid, p.create_time);
            alive.insert(key);
            let (cpu, io_r, io_w) = match (self.prev.get(&key), elapsed > 0.0) {
                (Some(prev), true) => {
                    let dt = (p.cpu_time - prev.cpu_time).max(0) as f64 / 10_000_000.0;
                    let cpu = (dt / (elapsed * self.ncpu as f64) * 100.0).clamp(0.0, 100.0);
                    let r = p.read_bytes.saturating_sub(prev.read) as f64 / elapsed;
                    let w = p.write_bytes.saturating_sub(prev.write) as f64 / elapsed;
                    (cpu, r, w)
                }
                _ => (0.0, 0.0, 0.0),
            };
            next_prev.insert(
                key,
                PrevSample {
                    cpu_time: p.cpu_time,
                    read: p.read_bytes,
                    write: p.write_bytes,
                },
            );

            let path = self
                .paths
                .entry(key)
                .or_insert_with(|| if p.pid > 4 { query_image_path(p.pid) } else { None })
                .clone();
            let kind = classify(p.pid, &p.name, p.session_id, path.as_deref());

            out.push(ProcessInfo {
                pid: p.pid,
                ppid: p.ppid,
                name: p.name,
                create_time: p.create_time,
                start_time: filetime_to_unix_ms(p.create_time),
                cpu: crate::util::round2(cpu),
                memory: if p.private_ws > 0 { p.private_ws } else { p.working_set },
                working_set: p.working_set,
                private_bytes: p.private_bytes,
                gpu: None,
                gpu_mem: None,
                io_read: io_r,
                io_write: io_w,
                threads: p.threads,
                handles: p.handles,
                session_id: p.session_id,
                path,
                kind,
                status: if kind == ProcKind::Critical {
                    "protected".into()
                } else {
                    "normal".into()
                },
                score: 0.0,
                whitelisted: false,
                ignored: false,
            });
        }
        self.prev = next_prev;
        self.paths.retain(|k, _| alive.contains(k));
        out
    }
}
