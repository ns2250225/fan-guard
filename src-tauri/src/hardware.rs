//! 系统级硬件信息：CPU、内存、显卡适配器（不依赖传感器的部分）

use crate::process::query_per_core_times;
use crate::util::{from_wide, wide};
use serde::Serialize;
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};
use windows_sys::Win32::System::SystemInformation::{
    GetLogicalProcessorInformation, GlobalMemoryStatusEx, MEMORYSTATUSEX, RelationProcessorCore,
    SYSTEM_LOGICAL_PROCESSOR_INFORMATION,
};
use windows_sys::Win32::System::Threading::{GetActiveProcessorCount, GetSystemTimes};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GpuAdapter {
    pub name: String,
    pub vendor: String,
    pub luid: String,
    pub dedicated_total: u64,
    pub shared_total: u64,
    pub integrated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StaticInfo {
    pub cpu_model: String,
    pub cpu_vendor: String,
    pub cores: u32,
    pub threads: u32,
    pub base_mhz: Option<f64>,
    pub total_memory: u64,
    pub gpus: Vec<GpuAdapter>,
    pub os_name: String,
    pub os_build: String,
    pub hostname: String,
}

pub(crate) fn reg_string(subkey: &str, value: &str) -> Option<String> {
    let sk = wide(subkey);
    let vn = wide(value);
    let mut buf = vec![0u16; 512];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            sk.as_ptr(),
            vn.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut _,
            &mut size,
        )
    };
    if r != 0 {
        return None;
    }
    let s = from_wide(&buf).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn reg_dword(subkey: &str, value: &str) -> Option<u32> {
    let sk = wide(subkey);
    let vn = wide(value);
    let mut v: u32 = 0;
    let mut size = 4u32;
    let r = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            sk.as_ptr(),
            vn.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut v as *mut u32 as *mut _,
            &mut size,
        )
    };
    if r == 0 {
        Some(v)
    } else {
        None
    }
}

fn physical_cores() -> u32 {
    unsafe {
        let mut len = 0u32;
        GetLogicalProcessorInformation(std::ptr::null_mut(), &mut len);
        if len == 0 {
            return 0;
        }
        let n = len as usize / std::mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION>() + 1;
        let mut buf: Vec<SYSTEM_LOGICAL_PROCESSOR_INFORMATION> = vec![std::mem::zeroed(); n];
        if GetLogicalProcessorInformation(buf.as_mut_ptr(), &mut len) == 0 {
            return 0;
        }
        let count = len as usize / std::mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION>();
        buf[..count]
            .iter()
            .filter(|i| i.Relationship == RelationProcessorCore)
            .count() as u32
    }
}

fn enum_gpus() -> Vec<GpuAdapter> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE};
    let mut out: Vec<GpuAdapter> = vec![];
    unsafe {
        let factory: IDXGIFactory1 = match CreateDXGIFactory1() {
            Ok(f) => f,
            Err(_) => return out,
        };
        let mut i = 0u32;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            i += 1;
            let desc = match adapter.GetDesc1() {
                Ok(d) => d,
                Err(_) => continue,
            };
            if desc.Flags & (DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0 {
                continue;
            }
            let name = from_wide(&desc.Description);
            if name.contains("Microsoft Basic Render") {
                continue;
            }
            let luid = format!(
                "0x{:08X}_0x{:08X}",
                desc.AdapterLuid.HighPart as u32, desc.AdapterLuid.LowPart
            );
            if out.iter().any(|g| g.luid == luid) {
                continue;
            }
            let vendor = match desc.VendorId {
                0x10DE => "NVIDIA",
                0x1002 | 0x1022 => "AMD",
                0x8086 => "Intel",
                0x5143 => "Qualcomm",
                _ => "Other",
            }
            .to_string();
            let dedicated = desc.DedicatedVideoMemory as u64;
            out.push(GpuAdapter {
                integrated: vendor == "Intel" || dedicated < 1024 * 1024 * 1024,
                name,
                vendor,
                luid,
                dedicated_total: dedicated,
                shared_total: desc.SharedSystemMemory as u64,
            });
        }
    }
    // 独立显卡排在前面
    out.sort_by_key(|g| (g.integrated, std::cmp::Reverse(g.dedicated_total)));
    out
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct ProcessorPowerInformation {
    number: u32,
    max_mhz: u32,
    current_mhz: u32,
    mhz_limit: u32,
    max_idle_state: u32,
    current_idle_state: u32,
}

#[link(name = "powrprof")]
extern "system" {
    fn CallNtPowerInformation(level: i32, input: *const std::ffi::c_void, in_len: u32, output: *mut std::ffi::c_void, out_len: u32) -> i32;
}

fn base_mhz(threads: u32) -> Option<f64> {
    let n = threads.max(1) as usize;
    let mut buf = vec![ProcessorPowerInformation::default(); n];
    let r = unsafe {
        CallNtPowerInformation(
            11, // ProcessorInformation
            std::ptr::null(),
            0,
            buf.as_mut_ptr() as *mut _,
            (n * std::mem::size_of::<ProcessorPowerInformation>()) as u32,
        )
    };
    if r != 0 {
        return reg_dword("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0", "~MHz").map(|v| v as f64);
    }
    buf.first().map(|p| p.max_mhz as f64).filter(|v| *v > 0.0)
}

pub fn total_memory() -> u64 {
    memory_status().map(|m| m.total).unwrap_or(0)
}

pub fn load_static_info() -> StaticInfo {
    let cpu_key = "HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0";
    let threads = unsafe { GetActiveProcessorCount(0xFFFF) };
    let os_key = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion";
    let build = reg_string(os_key, "CurrentBuild").unwrap_or_default();
    let mut os_name = reg_string(os_key, "ProductName").unwrap_or_else(|| "Windows".into());
    // Windows 11 的 ProductName 仍写作 Windows 10
    if build.parse::<u32>().unwrap_or(0) >= 22000 {
        os_name = os_name.replace("Windows 10", "Windows 11");
    }
    let display = reg_string(os_key, "DisplayVersion").unwrap_or_default();
    StaticInfo {
        cpu_model: reg_string(cpu_key, "ProcessorNameString").unwrap_or_else(|| "未知处理器".into()),
        cpu_vendor: reg_string(cpu_key, "VendorIdentifier").unwrap_or_default(),
        cores: physical_cores().max(1),
        threads: threads.max(1),
        base_mhz: base_mhz(threads),
        total_memory: total_memory(),
        gpus: enum_gpus(),
        os_name,
        os_build: if display.is_empty() { build } else { format!("{} (Build {})", display, build) },
        hostname: std::env::var("COMPUTERNAME").unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------
// 内存
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStatus {
    pub total: u64,
    pub used: u64,
    pub available: u64,
    pub usage: f64,
    pub commit_total: u64,
    pub commit_limit: u64,
    pub pagefile_usage: Option<f64>,
    pub cached: u64,
}

pub fn memory_status() -> Option<MemoryStatus> {
    unsafe {
        let mut m: MEMORYSTATUSEX = std::mem::zeroed();
        m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut m) == 0 {
            return None;
        }
        let mut pi: PERFORMANCE_INFORMATION = std::mem::zeroed();
        pi.cb = std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32;
        let (commit_total, commit_limit, cached) = if GetPerformanceInfo(&mut pi, pi.cb) != 0 {
            let ps = pi.PageSize as u64;
            (pi.CommitTotal as u64 * ps, pi.CommitLimit as u64 * ps, pi.SystemCache as u64 * ps)
        } else {
            (m.ullTotalPageFile - m.ullAvailPageFile, m.ullTotalPageFile, 0)
        };
        let used = m.ullTotalPhys.saturating_sub(m.ullAvailPhys);
        Some(MemoryStatus {
            total: m.ullTotalPhys,
            used,
            available: m.ullAvailPhys,
            usage: if m.ullTotalPhys > 0 {
                used as f64 / m.ullTotalPhys as f64 * 100.0
            } else {
                0.0
            },
            commit_total,
            commit_limit,
            pagefile_usage: None,
            cached,
        })
    }
}

// ---------------------------------------------------------------------------
// CPU 总使用率与每核心使用率
// ---------------------------------------------------------------------------

fn ft(f: &FILETIME) -> i64 {
    ((f.dwHighDateTime as i64) << 32) | f.dwLowDateTime as i64
}

pub struct CpuSampler {
    prev_total: Option<(i64, i64, i64)>,
    prev_cores: Vec<(i64, i64)>,
    threads: usize,
}

impl CpuSampler {
    pub fn new(threads: usize) -> Self {
        let mut s = Self {
            prev_total: None,
            prev_cores: vec![],
            threads,
        };
        s.sample();
        s
    }

    /// 返回 (总使用率, 每核心使用率)
    pub fn sample(&mut self) -> (f64, Vec<f64>) {
        let mut total = 0.0;
        unsafe {
            let mut idle: FILETIME = std::mem::zeroed();
            let mut kernel: FILETIME = std::mem::zeroed();
            let mut user: FILETIME = std::mem::zeroed();
            if GetSystemTimes(&mut idle, &mut kernel, &mut user) != 0 {
                let cur = (ft(&idle), ft(&kernel), ft(&user));
                if let Some(p) = self.prev_total {
                    let di = (cur.0 - p.0) as f64;
                    let dt = ((cur.1 - p.1) + (cur.2 - p.2)) as f64;
                    if dt > 0.0 {
                        total = ((1.0 - di / dt) * 100.0).clamp(0.0, 100.0);
                    }
                }
                self.prev_total = Some(cur);
            }
        }
        let cores_now = query_per_core_times(self.threads);
        let mut cores = Vec::with_capacity(cores_now.len());
        if cores_now.len() == self.prev_cores.len() {
            for (c, p) in cores_now.iter().zip(self.prev_cores.iter()) {
                let di = (c.0 - p.0) as f64;
                let dt = (c.1 - p.1) as f64;
                cores.push(if dt > 0.0 {
                    crate::util::round1(((1.0 - di / dt) * 100.0).clamp(0.0, 100.0))
                } else {
                    0.0
                });
            }
        } else {
            cores = vec![0.0; cores_now.len()];
        }
        self.prev_cores = cores_now;
        (total, cores)
    }
}
