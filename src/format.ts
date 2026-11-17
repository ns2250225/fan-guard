// 格式化与状态颜色工具

export function fmtBytes(b: number | null | undefined, digits = 1): string {
  if (b === null || b === undefined || isNaN(b)) return "-";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let v = b;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${i === 0 ? v.toFixed(0) : v.toFixed(digits)} ${units[i]}`;
}

export function fmtRate(b: number | null | undefined): string {
  if (b === null || b === undefined) return "-";
  if (b < 1) return "0 B/s";
  return `${fmtBytes(b)}/s`;
}

export function fmtPct(v: number | null | undefined, digits = 0): string {
  if (v === null || v === undefined || isNaN(v)) return "-";
  return `${v.toFixed(digits)}%`;
}

export function fmtTemp(v: number | null | undefined): string {
  if (v === null || v === undefined) return "暂不支持读取";
  return `${v.toFixed(0)}°C`;
}

export function fmtRpm(v: number | null | undefined): string {
  if (v === null || v === undefined) return "暂不支持读取";
  return `${Math.round(v)} RPM`;
}

export function fmtWatt(v: number | null | undefined): string {
  if (v === null || v === undefined) return "暂不支持读取";
  return `${v.toFixed(1)} W`;
}

export function fmtMhz(v: number | null | undefined): string {
  if (v === null || v === undefined) return "-";
  return v >= 1000 ? `${(v / 1000).toFixed(2)} GHz` : `${v.toFixed(0)} MHz`;
}

export function fmtDuration(secs: number | null | undefined): string {
  if (secs === null || secs === undefined) return "-";
  const s = Math.max(0, Math.floor(secs));
  if (s < 60) return `${s} 秒`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} 分 ${s % 60} 秒`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h} 小时 ${m % 60} 分`;
  return `${Math.floor(h / 24)} 天 ${h % 24} 小时`;
}

export function fmtUptime(startMs: number, now = Date.now()): string {
  if (!startMs) return "-";
  return fmtDuration((now - startMs) / 1000);
}

export function fmtTime(ms: number | null | undefined, withDate = true): string {
  if (!ms) return "-";
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  const t = `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
  if (!withDate) return t;
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${t}`;
}

export type Status = "normal" | "notice" | "warning" | "danger" | "unknown";

/** 负载状态：正常 / 注意 / 警告 / 危险 */
export function loadStatus(v: number | null | undefined, threshold: number, sustainedHigh = false): Status {
  if (v === null || v === undefined) return "unknown";
  if (v >= 98 && sustainedHigh) return "danger";
  if (v >= threshold || sustainedHigh) return "warning";
  if (v >= threshold * 0.75) return "notice";
  return "normal";
}

/** 温度状态：根据告警阈值划分 */
export function tempStatus(v: number | null | undefined, threshold: number): Status {
  if (v === null || v === undefined) return "unknown";
  if (v >= threshold) return "danger";
  if (v >= threshold - 10) return "warning";
  if (v >= threshold - 20) return "notice";
  return "normal";
}

export function fanStatus(v: number | null | undefined, threshold: number): Status {
  if (v === null || v === undefined) return "unknown";
  if (v >= threshold * 1.15) return "danger";
  if (v >= threshold) return "warning";
  if (v >= threshold * 0.75) return "notice";
  return "normal";
}

export function worst(...s: Status[]): Status {
  const rank: Record<Status, number> = { unknown: -1, normal: 0, notice: 1, warning: 2, danger: 3 };
  return s.reduce((a, b) => (rank[b] > rank[a] ? b : a), "unknown" as Status);
}

export const statusText: Record<Status, string> = {
  normal: "正常",
  notice: "注意",
  warning: "警告",
  danger: "危险",
  unknown: "无数据",
};

export const statusColor: Record<Status, string> = {
  normal: "var(--ok)",
  notice: "var(--notice)",
  warning: "var(--warn)",
  danger: "var(--danger)",
  unknown: "var(--muted-foreground)",
};

export const levelColor: Record<string, string> = {
  normal: "var(--ok)",
  notice: "var(--notice)",
  attention: "var(--notice)",
  high: "var(--warn)",
  critical: "var(--danger)",
};

export const levelText: Record<string, string> = {
  normal: "正常",
  notice: "注意",
  high: "高",
  critical: "严重",
};

export function gradeColor(score: number): string {
  if (score >= 80) return "var(--danger)";
  if (score >= 60) return "var(--warn)";
  if (score >= 40) return "var(--notice)";
  return "var(--ok)";
}

export function gradeText(score: number): string {
  if (score >= 80) return "重点排查";
  if (score >= 60) return "高负载";
  if (score >= 40) return "需要关注";
  return "正常";
}

export function cssVar(name: string): string {
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}
