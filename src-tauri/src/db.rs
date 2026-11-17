//! SQLite 历史数据库

use crate::util::format_local;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::io::Write;
use std::path::Path;

pub struct Db {
    conn: Connection,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareRow {
    pub timestamp: i64,
    pub cpu_usage: Option<f64>,
    pub cpu_temp: Option<f64>,
    pub cpu_power: Option<f64>,
    pub gpu_usage: Option<f64>,
    pub gpu_temp: Option<f64>,
    pub gpu_power: Option<f64>,
    pub memory_usage: Option<f64>,
    pub fan_rpm: Option<f64>,
    pub disk_temp: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventRow {
    pub id: i64,
    pub timestamp: i64,
    pub ended_at: Option<i64>,
    pub level: String,
    pub event_type: String,
    pub description: String,
    pub related_pid: Option<i64>,
    pub related_name: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionRow {
    pub id: i64,
    pub timestamp: i64,
    pub pid: i64,
    pub process_name: String,
    pub action: String,
    pub result: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessAggRow {
    pub process_name: String,
    pub avg_cpu: f64,
    pub max_cpu: f64,
    pub avg_memory: f64,
    pub avg_gpu: f64,
    pub samples: i64,
    pub last_seen: i64,
}

pub struct ProcessHistoryItem {
    pub pid: u32,
    pub start_time: i64,
    pub name: String,
    pub cpu: f64,
    pub memory: f64,
    pub gpu: Option<f64>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let conn = Connection::open(path).map_err(|e| e.to_string())?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            CREATE TABLE IF NOT EXISTS hardware_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp INTEGER NOT NULL,
                cpu_usage REAL,
                cpu_temp REAL,
                cpu_power REAL,
                gpu_usage REAL,
                gpu_temp REAL,
                gpu_power REAL,
                memory_usage REAL,
                fan_rpm REAL
            );
            CREATE INDEX IF NOT EXISTS idx_hw_ts ON hardware_history(timestamp);

            -- 进程历史：PID 会被系统重用，因此以 (pid, process_start) 作为进程实例标识
            CREATE TABLE IF NOT EXISTS process_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp INTEGER NOT NULL,
                pid INTEGER NOT NULL,
                process_start INTEGER NOT NULL DEFAULT 0,
                process_name TEXT NOT NULL,
                cpu_usage REAL,
                memory_usage REAL,
                gpu_usage REAL
            );
            CREATE INDEX IF NOT EXISTS idx_ph_ts ON process_history(timestamp);

            CREATE TABLE IF NOT EXISTS diagnostic_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp INTEGER NOT NULL,
                ended_at INTEGER,
                level TEXT NOT NULL,
                event_type TEXT NOT NULL,
                description TEXT NOT NULL,
                related_pid INTEGER,
                related_name TEXT,
                status TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_de_ts ON diagnostic_events(timestamp);

            CREATE TABLE IF NOT EXISTS process_actions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp INTEGER NOT NULL,
                pid INTEGER NOT NULL,
                process_name TEXT NOT NULL,
                action TEXT NOT NULL,
                result TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_pa_ts ON process_actions(timestamp);
            "#,
        )
        .map_err(|e| e.to_string())?;
        // 旧版本数据库升级：增加磁盘温度列（已存在时忽略错误）
        let _ = conn.execute("ALTER TABLE hardware_history ADD COLUMN disk_temp REAL", []);
        // 上次运行遗留的进行中事件标记为已结束
        let _ = conn.execute(
            "UPDATE diagnostic_events SET status = 'resolved', ended_at = COALESCE(ended_at, timestamp) WHERE status = 'active'",
            [],
        );
        Ok(Self { conn })
    }

    pub fn insert_hardware(&self, ts: i64, r: &HardwareRow) {
        let _ = self.conn.execute(
            "INSERT INTO hardware_history (timestamp, cpu_usage, cpu_temp, cpu_power, gpu_usage, gpu_temp, gpu_power, memory_usage, fan_rpm, disk_temp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![ts, r.cpu_usage, r.cpu_temp, r.cpu_power, r.gpu_usage, r.gpu_temp, r.gpu_power, r.memory_usage, r.fan_rpm, r.disk_temp],
        );
    }

    pub fn insert_process_history(&mut self, ts: i64, items: &[ProcessHistoryItem]) {
        let tx = match self.conn.transaction() {
            Ok(t) => t,
            Err(_) => return,
        };
        for p in items {
            let _ = tx.execute(
                "INSERT INTO process_history (timestamp, pid, process_start, process_name, cpu_usage, memory_usage, gpu_usage)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![ts, p.pid, p.start_time, p.name, p.cpu, p.memory, p.gpu],
            );
        }
        let _ = tx.commit();
    }

    pub fn insert_event(
        &self,
        ts: i64,
        level: &str,
        event_type: &str,
        description: &str,
        pid: Option<u32>,
        name: Option<&str>,
        status: &str,
    ) -> i64 {
        let _ = self.conn.execute(
            "INSERT INTO diagnostic_events (timestamp, level, event_type, description, related_pid, related_name, status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![ts, level, event_type, description, pid, name, status],
        );
        self.conn.last_insert_rowid()
    }

    pub fn update_event(&self, id: i64, status: &str, ended_at: Option<i64>, description: Option<&str>, level: Option<&str>) {
        let _ = self.conn.execute(
            "UPDATE diagnostic_events SET status = ?2,
                ended_at = COALESCE(?3, ended_at),
                description = COALESCE(?4, description),
                level = COALESCE(?5, level)
             WHERE id = ?1",
            params![id, status, ended_at, description, level],
        );
    }

    pub fn insert_action(&self, ts: i64, pid: u32, name: &str, action: &str, result: &str) {
        let _ = self.conn.execute(
            "INSERT INTO process_actions (timestamp, pid, process_name, action, result) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ts, pid, name, action, result],
        );
    }

    /// 按时间桶聚合查询硬件历史，控制返回点数
    pub fn query_hardware(&self, from: i64, to: i64, max_points: i64) -> Vec<HardwareRow> {
        let span = (to - from).max(1);
        let bucket = (span / max_points.max(1)).max(1000);
        let mut stmt = match self.conn.prepare(
            "SELECT (timestamp / ?3) * ?3 AS b, AVG(cpu_usage), AVG(cpu_temp), AVG(cpu_power), AVG(gpu_usage),
                    AVG(gpu_temp), AVG(gpu_power), AVG(memory_usage), AVG(fan_rpm), AVG(disk_temp)
             FROM hardware_history WHERE timestamp BETWEEN ?1 AND ?2 GROUP BY b ORDER BY b",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let rows = stmt.query_map(params![from, to, bucket], |r| {
            Ok(HardwareRow {
                timestamp: r.get(0)?,
                cpu_usage: r.get(1)?,
                cpu_temp: r.get(2)?,
                cpu_power: r.get(3)?,
                gpu_usage: r.get(4)?,
                gpu_temp: r.get(5)?,
                gpu_power: r.get(6)?,
                memory_usage: r.get(7)?,
                fan_rpm: r.get(8)?,
                disk_temp: r.get(9)?,
            })
        });
        match rows {
            Ok(it) => it.filter_map(|x| x.ok()).collect(),
            Err(_) => vec![],
        }
    }

    pub fn query_events(&self, from: i64, to: i64, limit: i64) -> Vec<EventRow> {
        let mut stmt = match self.conn.prepare(
            "SELECT id, timestamp, ended_at, level, event_type, description, related_pid, related_name, status
             FROM diagnostic_events WHERE timestamp BETWEEN ?1 AND ?2 ORDER BY timestamp DESC LIMIT ?3",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let rows = stmt.query_map(params![from, to, limit], |r| {
            Ok(EventRow {
                id: r.get(0)?,
                timestamp: r.get(1)?,
                ended_at: r.get(2)?,
                level: r.get(3)?,
                event_type: r.get(4)?,
                description: r.get(5)?,
                related_pid: r.get(6)?,
                related_name: r.get(7)?,
                status: r.get(8)?,
            })
        });
        match rows {
            Ok(it) => it.filter_map(|x| x.ok()).collect(),
            Err(_) => vec![],
        }
    }

    pub fn query_actions(&self, from: i64, to: i64, limit: i64) -> Vec<ActionRow> {
        let mut stmt = match self.conn.prepare(
            "SELECT id, timestamp, pid, process_name, action, result FROM process_actions
             WHERE timestamp BETWEEN ?1 AND ?2 ORDER BY timestamp DESC LIMIT ?3",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let rows = stmt.query_map(params![from, to, limit], |r| {
            Ok(ActionRow {
                id: r.get(0)?,
                timestamp: r.get(1)?,
                pid: r.get(2)?,
                process_name: r.get(3)?,
                action: r.get(4)?,
                result: r.get(5)?,
            })
        });
        match rows {
            Ok(it) => it.filter_map(|x| x.ok()).collect(),
            Err(_) => vec![],
        }
    }

    pub fn query_process_agg(&self, from: i64, to: i64, limit: i64) -> Vec<ProcessAggRow> {
        let mut stmt = match self.conn.prepare(
            "SELECT process_name, AVG(cpu_usage), MAX(cpu_usage), AVG(memory_usage), AVG(COALESCE(gpu_usage, 0)), COUNT(*), MAX(timestamp)
             FROM process_history WHERE timestamp BETWEEN ?1 AND ?2
             GROUP BY process_name ORDER BY AVG(cpu_usage) * COUNT(*) DESC LIMIT ?3",
        ) {
            Ok(s) => s,
            Err(_) => return vec![],
        };
        let rows = stmt.query_map(params![from, to, limit], |r| {
            Ok(ProcessAggRow {
                process_name: r.get(0)?,
                avg_cpu: r.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
                max_cpu: r.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
                avg_memory: r.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
                avg_gpu: r.get::<_, Option<f64>>(4)?.unwrap_or(0.0),
                samples: r.get(5)?,
                last_seen: r.get(6)?,
            })
        });
        match rows {
            Ok(it) => it.filter_map(|x| x.ok()).collect(),
            Err(_) => vec![],
        }
    }

    pub fn cleanup(&self, retention_days: u64) {
        let cutoff = crate::util::now_ms() - retention_days as i64 * 86_400_000;
        for table in ["hardware_history", "process_history", "diagnostic_events", "process_actions"] {
            let _ = self
                .conn
                .execute(&format!("DELETE FROM {} WHERE timestamp < ?1", table), params![cutoff]);
        }
    }

    pub fn clear_all(&self) -> Result<(), String> {
        self.conn
            .execute_batch(
                "DELETE FROM hardware_history; DELETE FROM process_history; DELETE FROM diagnostic_events; DELETE FROM process_actions; VACUUM;",
            )
            .map_err(|e| e.to_string())
    }

    /// 导出 CSV（UTF-8 BOM，Excel 可直接打开）
    pub fn export_csv(&self, kind: &str, from: i64, to: i64, path: &Path) -> Result<usize, String> {
        let mut f = std::fs::File::create(path).map_err(|e| format!("无法创建文件：{}", e))?;
        f.write_all(&[0xEF, 0xBB, 0xBF]).map_err(|e| e.to_string())?;
        let esc = |s: &str| {
            if s.contains(',') || s.contains('"') || s.contains('\n') {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.to_string()
            }
        };
        let opt = |v: Option<f64>| v.map(|x| format!("{:.2}", x)).unwrap_or_default();
        let mut count = 0usize;
        match kind {
            "hardware" => {
                writeln!(f, "时间,时间戳,CPU使用率(%),CPU温度(°C),CPU功耗(W),GPU使用率(%),GPU温度(°C),GPU功耗(W),内存使用率(%),风扇转速(RPM),磁盘温度(°C)")
                    .map_err(|e| e.to_string())?;
                let mut stmt = self
                    .conn
                    .prepare(
                        "SELECT timestamp, cpu_usage, cpu_temp, cpu_power, gpu_usage, gpu_temp, gpu_power, memory_usage, fan_rpm, disk_temp
                         FROM hardware_history WHERE timestamp BETWEEN ?1 AND ?2 ORDER BY timestamp",
                    )
                    .map_err(|e| e.to_string())?;
                let mut rows = stmt.query(params![from, to]).map_err(|e| e.to_string())?;
                while let Ok(Some(r)) = rows.next() {
                    let ts: i64 = r.get(0).unwrap_or(0);
                    let vals: Vec<String> = (1..=9).map(|i| opt(r.get::<_, Option<f64>>(i).unwrap_or(None))).collect();
                    writeln!(f, "{},{},{}", format_local(ts), ts, vals.join(",")).map_err(|e| e.to_string())?;
                    count += 1;
                }
            }
            "process" => {
                writeln!(f, "时间,时间戳,PID,进程启动时间,进程名称,CPU使用率(%),内存占用(MB),GPU使用率(%)").map_err(|e| e.to_string())?;
                let mut stmt = self
                    .conn
                    .prepare(
                        "SELECT timestamp, pid, process_start, process_name, cpu_usage, memory_usage, gpu_usage
                         FROM process_history WHERE timestamp BETWEEN ?1 AND ?2 ORDER BY timestamp",
                    )
                    .map_err(|e| e.to_string())?;
                let mut rows = stmt.query(params![from, to]).map_err(|e| e.to_string())?;
                while let Ok(Some(r)) = rows.next() {
                    let ts: i64 = r.get(0).unwrap_or(0);
                    let pid: i64 = r.get(1).unwrap_or(0);
                    let start: i64 = r.get(2).unwrap_or(0);
                    let name: String = r.get(3).unwrap_or_default();
                    let cpu = opt(r.get::<_, Option<f64>>(4).unwrap_or(None));
                    let mem = r
                        .get::<_, Option<f64>>(5)
                        .unwrap_or(None)
                        .map(|m| format!("{:.1}", m / 1024.0 / 1024.0))
                        .unwrap_or_default();
                    let gpu = opt(r.get::<_, Option<f64>>(6).unwrap_or(None));
                    writeln!(f, "{},{},{},{},{},{},{},{}", format_local(ts), ts, pid, format_local(start), esc(&name), cpu, mem, gpu)
                        .map_err(|e| e.to_string())?;
                    count += 1;
                }
            }
            "events" => {
                writeln!(f, "开始时间,结束时间,异常等级,异常类型,诊断说明,关联进程PID,关联进程,状态").map_err(|e| e.to_string())?;
                for e in self.query_events(from, to, i64::MAX) {
                    writeln!(
                        f,
                        "{},{},{},{},{},{},{},{}",
                        format_local(e.timestamp),
                        e.ended_at.map(format_local).unwrap_or_default(),
                        esc(&e.level),
                        esc(&e.event_type),
                        esc(&e.description),
                        e.related_pid.map(|p| p.to_string()).unwrap_or_default(),
                        esc(&e.related_name.unwrap_or_default()),
                        esc(&e.status)
                    )
                    .map_err(|e| e.to_string())?;
                    count += 1;
                }
            }
            "actions" => {
                writeln!(f, "时间,PID,进程名称,操作类型,执行结果").map_err(|e| e.to_string())?;
                for a in self.query_actions(from, to, i64::MAX) {
                    writeln!(
                        f,
                        "{},{},{},{},{}",
                        format_local(a.timestamp),
                        a.pid,
                        esc(&a.process_name),
                        esc(&a.action),
                        esc(&a.result)
                    )
                    .map_err(|e| e.to_string())?;
                    count += 1;
                }
            }
            _ => return Err("未知的导出类型".into()),
        }
        Ok(count)
    }
}
