//! 通用工具函数

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Windows FILETIME（1601 年起的 100ns 计数）转换为 Unix 毫秒
pub fn filetime_to_unix_ms(ft: i64) -> i64 {
    if ft <= 0 {
        return 0;
    }
    (ft - 116_444_736_000_000_000) / 10_000
}

pub fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// %SystemRoot%，通常为 C:\Windows
pub fn system_root() -> String {
    std::env::var("SystemRoot")
        .or_else(|_| std::env::var("windir"))
        .unwrap_or_else(|_| "C:\\Windows".into())
}

pub fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

pub fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// 将 Unix 毫秒转换为本地时间字符串
pub fn format_local(ms: i64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_millis_opt(ms) {
        chrono::LocalResult::Single(t) => t.format("%Y-%m-%d %H:%M:%S").to_string(),
        _ => String::new(),
    }
}

pub fn format_bytes(b: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{:.0} {}", v, UNITS[i])
    } else {
        format!("{:.1} {}", v, UNITS[i])
    }
}

/// 后台运行子进程时不弹出控制台窗口
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 进程创建时间为 64 位 FILETIME（约 1.3e17），超出 JavaScript 安全整数范围，
/// 因此在前后端之间以字符串传递，避免精度丢失导致进程身份校验失败
pub fn ser_i64_str<S: serde::Serializer>(v: &i64, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(&v.to_string())
}

pub fn parse_create_time(s: &str) -> Result<i64, String> {
    s.trim().parse::<i64>().map_err(|_| "无效的进程创建时间".to_string())
}
