//! HWiNFO 共享内存传感器读取
//!
//! HWiNFO 在设置中启用「Shared Memory Support（共享内存支持）」后，会把全部传感器
//! 写入名为 `Global\HWiNFO_SENS_SM2` 的共享内存。本模块只读这块内存：
//! - 不需要管理员权限（HWiNFO 自身负责驱动与底层访问）；
//! - 可在 LibreHardwareMonitor 读取不到 CPU 温度、风扇转速时作为数据来源。
//!
//! 内存布局（#pragma pack(1)）：
//!   头部：Signature u32 | Version u32 | Revision u32 | PollTime i64 |
//!         SensorOffset u32 | SensorSize u32 | SensorCount u32 |
//!         ReadingOffset u32 | ReadingSize u32 | ReadingCount u32
//!   传感器：Id u32 | Instance u32 | NameOrig char[128] | NameUser char[128] | (v2+) NameUtf8[128]
//!   读数：  Type u32 | SensorIndex u32 | ReadingId u32 | LabelOrig char[128] | LabelUser char[128] |
//!           Unit char[16] | Value f64 | Min f64 | Max f64 | Avg f64 | (v2+) LabelUtf8[128] | UnitUtf8[16]

use crate::sensors::{FanReading, GpuSensor, HwNode, SensorNode, SensorSummary, TempReading};
use crate::util::wide;
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Memory::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, VirtualQuery, FILE_MAP_READ, MEMORY_BASIC_INFORMATION,
};

const SIG_ACTIVE: u32 = 0x5369_5748; // "HWiS"
const SIG_DEAD: u32 = 0x4441_4544; // "DEAD"：HWiNFO 已关闭共享内存

pub struct HwinfoData {
    pub nodes: Vec<HwNode>,
    /// HWiNFO 最近一次轮询时间（Unix 秒）
    pub poll_time: i64,
}

/// 读取失败原因：notFound（HWiNFO 未运行或未启用共享内存）/ inactive / invalid
pub fn read() -> Result<HwinfoData, &'static str> {
    let data = unsafe {
        let mut h = std::ptr::null_mut();
        for name in ["Global\\HWiNFO_SENS_SM2", "HWiNFO_SENS_SM2"] {
            let w = wide(name);
            h = OpenFileMappingW(FILE_MAP_READ, 0, w.as_ptr());
            if !h.is_null() {
                break;
            }
        }
        if h.is_null() {
            return Err("notFound");
        }
        let view = MapViewOfFile(h, FILE_MAP_READ, 0, 0, 0);
        if view.Value.is_null() {
            CloseHandle(h);
            return Err("notFound");
        }
        let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
        let n = VirtualQuery(view.Value, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>());
        let len = if n == 0 { 0 } else { mbi.RegionSize };
        // 复制一份后立即释放映射，避免长时间占用
        let bytes = if len > 0 {
            std::slice::from_raw_parts(view.Value as *const u8, len).to_vec()
        } else {
            vec![]
        };
        UnmapViewOfFile(view);
        CloseHandle(h);
        bytes
    };
    parse(&data)
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn i64_at(d: &[u8], o: usize) -> Option<i64> {
    d.get(o..o + 8).map(|b| i64::from_le_bytes(b.try_into().unwrap()))
}

fn f64_at(d: &[u8], o: usize) -> Option<f64> {
    d.get(o..o + 8)
        .map(|b| f64::from_le_bytes(b.try_into().unwrap()))
        .filter(|v| v.is_finite())
}

fn cstr(d: &[u8], o: usize, len: usize) -> String {
    match d.get(o..o + len) {
        Some(b) => {
            let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
            String::from_utf8_lossy(&b[..end]).trim().to_string()
        }
        None => String::new(),
    }
}

/// 根据 HWiNFO 传感器名称推断硬件类型（与 LibreHardwareMonitor 的类型名保持一致）
fn classify(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if n.starts_with("cpu [") || n.starts_with("cpu[") {
        "Cpu"
    } else if n.starts_with("gpu [") || n.starts_with("gpu[") || n.starts_with("dgpu [") || n.starts_with("igpu [") {
        "GpuHwinfo"
    } else if n.starts_with("s.m.a.r.t") || n.starts_with("drive:") || n.starts_with("drive [") {
        "Storage"
    } else if n.starts_with("system:") || n.starts_with("memory timings") || n.starts_with("ddr") {
        "Memory"
    } else if n.contains("battery") {
        "Battery"
    } else if n.starts_with("network") {
        "Network"
    } else if n.starts_with("windows hardware errors") {
        "Other"
    } else {
        // 主板 Super I/O、嵌入式控制器（EC）、笔记本厂商传感器等
        "Motherboard"
    }
}

/// 去掉 "CPU [#0]: " / "S.M.A.R.T.: " 之类的前缀与 ": Enhanced" 之类的后缀
pub fn clean_name(name: &str) -> String {
    let mut s = name.trim();
    if let Some(p) = s.find("]: ") {
        s = &s[p + 3..];
    } else {
        for prefix in ["S.M.A.R.T.: ", "Drive: "] {
            if let Some(r) = s.strip_prefix(prefix) {
                s = r;
            }
        }
    }
    s.split(": ").next().unwrap_or(s).trim_end_matches(':').trim().to_string()
}

fn parse(d: &[u8]) -> Result<HwinfoData, &'static str> {
    let sig = u32_at(d, 0).ok_or("invalid")?;
    if sig == SIG_DEAD {
        return Err("inactive");
    }
    if sig != SIG_ACTIVE {
        return Err("invalid");
    }
    let poll_time = i64_at(d, 12).unwrap_or(0);
    let g = |o: usize| u32_at(d, o).map(|v| v as usize).ok_or("invalid");
    let (so, ss, sn) = (g(20)?, g(24)?, g(28)?);
    let (ro, rs, rn) = (g(32)?, g(36)?, g(40)?);
    if ss < 264 || rs < 316 || sn > 4096 || rn > 65536 {
        return Err("invalid");
    }

    let mut nodes: Vec<HwNode> = Vec::with_capacity(sn);
    for i in 0..sn {
        let b = so + i * ss;
        if b + 264 > d.len() {
            break;
        }
        let id = u32_at(d, b).unwrap_or(0);
        let inst = u32_at(d, b + 4).unwrap_or(0);
        let name = cstr(d, b + 8, 128);
        nodes.push(HwNode {
            id: format!("/hwinfo/{:x}/{}/{}", id, inst, i),
            hw_type: classify(&name).into(),
            name,
            parent: None,
            parent_type: None,
            parent_name: None,
            sensors: vec![],
        });
    }

    for j in 0..rn {
        let b = ro + j * rs;
        if b + 316 > d.len() {
            break;
        }
        let stype = match u32_at(d, b).unwrap_or(0) {
            1 => "Temperature",
            2 => "Voltage",
            3 => "Fan",
            4 => "Current",
            5 => "Power",
            6 => "Clock",
            7 => "Load",
            8 => "Other",
            _ => continue,
        };
        let idx = u32_at(d, b + 4).unwrap_or(u32::MAX) as usize;
        let rid = u32_at(d, b + 8).unwrap_or(0);
        let label = cstr(d, b + 12, 128);
        let unit = cstr(d, b + 268, 16);
        let mut value = f64_at(d, b + 284);
        let mut min = f64_at(d, b + 292);
        let mut max = f64_at(d, b + 300);
        let unit = if stype == "Temperature" {
            // 用户在 HWiNFO 中选择了华氏度时换算为摄氏度
            if unit.contains('F') {
                let c = |v: Option<f64>| v.map(|f| (f - 32.0) * 5.0 / 9.0);
                value = c(value);
                min = c(min);
                max = c(max);
            }
            None
        } else if unit.is_empty() || !unit.is_ascii() {
            None
        } else {
            Some(unit)
        };
        let node = match nodes.get_mut(idx) {
            Some(n) => n,
            None => continue,
        };
        node.sensors.push(SensorNode {
            id: format!("{}/{:x}", node.id, rid),
            name: label,
            sensor_type: stype.into(),
            value,
            min,
            max,
            unit,
        });
    }
    nodes.retain(|n| !n.sensors.is_empty());
    Ok(HwinfoData { nodes, poll_time })
}

// ---------------------------------------------------------------------------
// 汇总（HWiNFO 的传感器命名与 LibreHardwareMonitor 不同，需要单独匹配）
// ---------------------------------------------------------------------------

fn valid_temp(v: Option<f64>) -> Option<f64> {
    v.filter(|t| *t > 1.0 && *t < 150.0)
}

/// 「距离 TjMAX」类读数单位也是温度，但不是实际温度，必须排除
fn is_real_temp(name: &str) -> bool {
    let n = name.to_lowercase();
    !n.contains("distance") && !n.contains("tjmax") && !n.contains("limit") && !n.contains("throttl")
}

fn find(hw: &HwNode, stype: &str, names: &[&str]) -> Option<f64> {
    for n in names {
        if let Some(v) = hw
            .sensors
            .iter()
            .find(|s| s.sensor_type == stype && s.name.eq_ignore_ascii_case(n))
            .and_then(|s| s.value)
        {
            return Some(v);
        }
    }
    None
}

fn find_contains(hw: &HwNode, stype: &str, needles: &[&str]) -> Option<f64> {
    for k in needles {
        if let Some(v) = hw
            .sensors
            .iter()
            .find(|s| s.sensor_type == stype && s.name.to_lowercase().contains(k))
            .and_then(|s| s.value)
        {
            return Some(v);
        }
    }
    None
}

fn max_of<'a>(it: impl Iterator<Item = &'a SensorNode>) -> Option<f64> {
    it.filter_map(|s| s.value).fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))))
}

pub fn summarize(nodes: &[HwNode]) -> SensorSummary {
    let mut s = SensorSummary::default();

    for hw in nodes {
        let t = hw.hw_type.as_str();
        let label = clean_name(&hw.name);

        if t == "Cpu" {
            if s.cpu_temp.is_none() {
                s.cpu_temp = valid_temp(find(
                    hw,
                    "Temperature",
                    &[
                        "CPU Package",
                        "CPU (Tctl/Tdie)",
                        "CPU Die (average)",
                        "CPU (Tdie)",
                        "CPU (Tctl)",
                        "CPU Package (PECI)",
                        "Core Max",
                        "CPU",
                    ],
                ))
                .or_else(|| {
                    valid_temp(max_of(
                        hw.sensors.iter().filter(|x| x.sensor_type == "Temperature" && is_real_temp(&x.name)),
                    ))
                });
            }
            if s.cpu_power.is_none() {
                s.cpu_power = find(
                    hw,
                    "Power",
                    &["CPU Package Power", "CPU PPT", "CPU Power", "CPU Core Power (SVI2 TFN)", "IA Cores Power"],
                )
                .or_else(|| find_contains(hw, "Power", &["package"]));
            }
            if s.cpu_core_clocks.is_empty() {
                let clocks: Vec<f64> = hw
                    .sensors
                    .iter()
                    .filter(|x| {
                        let n = x.name.to_lowercase();
                        x.sensor_type == "Clock"
                            && n.contains("core")
                            && n.contains("clock")
                            && !n.contains("effective")
                            && !n.contains("avg")
                            && !n.contains("uncore")
                            && !n.contains("ring")
                            && !n.contains("bus")
                    })
                    .filter_map(|x| x.value)
                    .filter(|v| *v > 0.0)
                    .collect();
                if !clocks.is_empty() {
                    s.cpu_core_clocks = clocks;
                }
            }
            if s.cpu_clock.is_none() {
                s.cpu_clock = find(hw, "Clock", &["Core Clocks (avg)", "Core Clocks"]);
            }
        } else if t.starts_with("Gpu") {
            let mem_used = find(hw, "Other", &["GPU Memory Allocated", "GPU D3D Memory Dedicated", "GPU Memory Used"])
                .or_else(|| find_contains(hw, "Other", &["memory allocated", "memory used"]));
            let g = GpuSensor {
                name: label.clone(),
                temp: valid_temp(
                    find(hw, "Temperature", &["GPU Temperature", "GPU Core Temperature", "GPU Thermal Diode"]).or_else(|| {
                        hw.sensors
                            .iter()
                            .find(|x| {
                                let n = x.name.to_lowercase();
                                x.sensor_type == "Temperature" && !n.contains("hot") && !n.contains("memory") && is_real_temp(&n)
                            })
                            .and_then(|x| x.value)
                    }),
                ),
                hotspot: valid_temp(find_contains(hw, "Temperature", &["hot spot", "hotspot"])),
                power: find(
                    hw,
                    "Power",
                    &["GPU Power", "Total Board Power (TBP)", "Total Graphics Power", "GPU ASIC Power", "GPU Core Power", "GPU Total Power"],
                )
                .or_else(|| hw.sensors.iter().find(|x| x.sensor_type == "Power").and_then(|x| x.value)),
                clock: find(hw, "Clock", &["GPU Clock", "GPU Core Clock"]),
                mem_clock: find(hw, "Clock", &["GPU Memory Clock"]),
                load: find(hw, "Load", &["GPU Core Load", "GPU Utilization", "GPU D3D Usage", "GPU Load"]),
                mem_used: mem_used.map(|mb| (mb * 1024.0 * 1024.0) as u64),
                mem_total: None,
                fan_rpm: max_of(hw.sensors.iter().filter(|x| x.sensor_type == "Fan")),
                fan_percent: hw
                    .sensors
                    .iter()
                    .find(|x| x.sensor_type == "Load" && x.name.to_lowercase().contains("fan"))
                    .and_then(|x| x.value),
            };
            let useful = g.temp.is_some() || g.power.is_some() || g.load.is_some() || g.fan_rpm.is_some();
            if useful {
                s.gpus.push(g);
            }
        }

        // 风扇
        for sensor in hw.sensors.iter().filter(|x| x.sensor_type == "Fan") {
            let v = match sensor.value {
                Some(v) => v,
                None => continue,
            };
            if v <= 0.0 && sensor.max.unwrap_or(0.0) <= 0.0 {
                continue;
            }
            let lname = sensor.name.to_lowercase();
            let kind = if t.starts_with("Gpu") || lname.contains("gpu") {
                "gpu"
            } else if lname.contains("cpu") || lname.contains("pump") || lname.contains("aio") || lname.contains("opt") {
                "cpu"
            } else {
                "case"
            };
            s.fans.push(FanReading {
                name: sensor.name.clone(),
                hardware: label.clone(),
                kind: kind.into(),
                rpm: v,
            });
        }

        // 温度
        for sensor in hw
            .sensors
            .iter()
            .filter(|x| x.sensor_type == "Temperature" && is_real_temp(&x.name))
        {
            if let Some(v) = valid_temp(sensor.value) {
                s.temps.push(TempReading {
                    name: sensor.name.clone(),
                    hardware: label.clone(),
                    hw_type: hw.hw_type.clone(),
                    value: v,
                    max: sensor.max,
                });
            }
        }
    }

    if s.cpu_clock.is_none() && !s.cpu_core_clocks.is_empty() {
        s.cpu_clock = Some(s.cpu_core_clocks.iter().sum::<f64>() / s.cpu_core_clocks.len() as f64);
    }
    // 笔记本 / 主板：CPU 温度可能只在 EC 或 Super I/O 传感器上提供
    if s.cpu_temp.is_none() {
        s.cpu_temp = s
            .temps
            .iter()
            .find(|t| t.hw_type == "Motherboard" && t.name.to_lowercase().contains("cpu"))
            .map(|t| t.value);
    }
    s
}
