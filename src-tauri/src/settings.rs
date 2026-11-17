//! 用户配置：保存在应用配置目录下的 settings.json

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    // ---------- 基础设置 ----------
    /// 开机自动启动
    pub autostart: bool,
    /// 关闭窗口时最小化到托盘
    pub minimize_to_tray: bool,
    /// 是否显示桌面通知
    pub notifications: bool,
    /// 是否自动启动硬件监控
    pub auto_start_monitoring: bool,
    /// 始终以管理员身份运行（启动时请求 UAC；读取笔记本 EC 风扇、CPU 温度需要）
    pub run_as_admin: bool,
    /// 主题：dark / light / system
    pub theme: String,

    // ---------- 监控设置 ----------
    /// 进程刷新间隔（毫秒）：500 / 1000 / 2000 / 5000
    pub process_interval_ms: u64,
    /// 传感器数据来源：auto（HWiNFO 优先 + LibreHardwareMonitor 补充）/ lhm / hwinfo
    pub sensor_source: String,
    /// 自动集成 HWiNFO：自动导入、配置、启动与续期
    pub hwinfo_auto: bool,
    /// 硬件传感器刷新间隔（毫秒）
    pub sensor_interval_ms: u64,
    /// 历史数据采样间隔（毫秒）
    pub history_interval_ms: u64,
    /// CPU 高负载阈值（%）
    pub cpu_threshold: f64,
    /// GPU 高负载阈值（%）
    pub gpu_threshold: f64,
    /// CPU 温度告警阈值（°C）
    pub cpu_temp_threshold: f64,
    /// GPU 温度告警阈值（°C）
    pub gpu_temp_threshold: f64,
    /// 磁盘温度告警阈值（°C）
    pub disk_temp_threshold: f64,
    /// 风扇转速告警阈值（RPM）
    pub fan_rpm_threshold: f64,
    /// 判定"持续"高负载所需的时间（秒）
    pub sustain_seconds: u64,
    /// 历史数据保留天数
    pub history_retention_days: u64,

    // ---------- 进程设置 ----------
    /// 进程白名单（只影响告警和诊断排序）
    pub whitelist: Vec<String>,
    /// 自动忽略的进程（不参与诊断与告警）
    pub ignore_list: Vec<String>,
    /// 是否显示系统进程
    pub show_system_processes: bool,
    /// 是否显示后台服务
    pub show_services: bool,
    /// 是否启用结束进程确认（强制结束、结束进程树始终需要确认）
    pub confirm_kill: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            autostart: false,
            minimize_to_tray: true,
            notifications: true,
            auto_start_monitoring: true,
            run_as_admin: false,
            theme: "dark".into(),
            process_interval_ms: 1000,
            sensor_source: "auto".into(),
            hwinfo_auto: true,
            sensor_interval_ms: 2000,
            history_interval_ms: 5000,
            cpu_threshold: 80.0,
            gpu_threshold: 85.0,
            cpu_temp_threshold: 90.0,
            gpu_temp_threshold: 85.0,
            disk_temp_threshold: 60.0,
            fan_rpm_threshold: 4000.0,
            sustain_seconds: 30,
            history_retention_days: 7,
            whitelist: vec![],
            ignore_list: vec![],
            show_system_processes: true,
            show_services: true,
            confirm_kill: true,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        let mut s: Settings = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        s.sanitize();
        s
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }

    /// 修正非法取值
    pub fn sanitize(&mut self) {
        self.process_interval_ms = self.process_interval_ms.clamp(500, 5000);
        self.sensor_interval_ms = self.sensor_interval_ms.clamp(500, 10000);
        self.history_interval_ms = self.history_interval_ms.clamp(1000, 60000);
        self.cpu_threshold = self.cpu_threshold.clamp(10.0, 100.0);
        self.gpu_threshold = self.gpu_threshold.clamp(10.0, 100.0);
        self.cpu_temp_threshold = self.cpu_temp_threshold.clamp(40.0, 110.0);
        self.gpu_temp_threshold = self.gpu_temp_threshold.clamp(40.0, 110.0);
        self.disk_temp_threshold = self.disk_temp_threshold.clamp(35.0, 90.0);
        self.fan_rpm_threshold = self.fan_rpm_threshold.clamp(500.0, 20000.0);
        self.sustain_seconds = self.sustain_seconds.clamp(5, 600);
        self.history_retention_days = self.history_retention_days.clamp(1, 90);
        if !matches!(self.sensor_source.as_str(), "auto" | "lhm" | "hwinfo") {
            self.sensor_source = "auto".into();
        }
        if !matches!(self.theme.as_str(), "dark" | "light" | "system") {
            self.theme = "dark".into();
        }
        let norm = |v: &mut Vec<String>| {
            let mut out: Vec<String> = v
                .iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            out.sort_by_key(|s| s.to_lowercase());
            out.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
            *v = out;
        };
        norm(&mut self.whitelist);
        norm(&mut self.ignore_list);
    }

    pub fn is_whitelisted(&self, name: &str) -> bool {
        self.whitelist.iter().any(|w| w.eq_ignore_ascii_case(name))
    }

    pub fn is_ignored(&self, name: &str) -> bool {
        self.ignore_list.iter().any(|w| w.eq_ignore_ascii_case(name))
    }
}

pub fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join("settings.json")
}
