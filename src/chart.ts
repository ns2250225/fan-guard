// ECharts 通用配置

import { cssVar, fmtTime } from "./format";

export interface SeriesDef {
  key: string;
  name: string;
  color: string;
  unit: string;
  axis: "pct" | "temp" | "rpm" | "watt";
}

export const SERIES: SeriesDef[] = [
  { key: "cpu", name: "CPU 使用率", color: "#3b82f6", unit: "%", axis: "pct" },
  { key: "gpu", name: "GPU 使用率", color: "#a855f7", unit: "%", axis: "pct" },
  { key: "mem", name: "内存使用率", color: "#14b8a6", unit: "%", axis: "pct" },
  { key: "cpuTemp", name: "CPU 温度", color: "#f97316", unit: "°C", axis: "temp" },
  { key: "gpuTemp", name: "GPU 温度", color: "#ef4444", unit: "°C", axis: "temp" },
  { key: "diskTemp", name: "磁盘温度", color: "#84cc16", unit: "°C", axis: "temp" },
  { key: "fan", name: "风扇转速", color: "#eab308", unit: "RPM", axis: "rpm" },
];

export function axisColors() {
  return {
    text: cssVar("--chart-text") || "#8b95a8",
    grid: cssVar("--chart-grid") || "rgba(148,163,184,0.12)",
    bg: cssVar("--popover") || "#161d2d",
    fg: cssVar("--foreground") || "#e5e9f0",
    border: cssVar("--border") || "#222b3d",
  };
}

export function tooltipBase() {
  const c = axisColors();
  return {
    trigger: "axis",
    backgroundColor: c.bg,
    borderColor: c.border,
    textStyle: { color: c.fg, fontSize: 12 },
    axisPointer: { type: "line", lineStyle: { color: c.text, type: "dashed" } },
  };
}

export function timeAxis() {
  const c = axisColors();
  return {
    type: "time",
    axisLine: { lineStyle: { color: c.grid } },
    axisLabel: {
      color: c.text,
      fontSize: 11,
      hideOverlap: true,
      formatter: (v: number) => {
        const d = new Date(v);
        const p = (n: number) => String(n).padStart(2, "0");
        return `${p(d.getHours())}:${p(d.getMinutes())}`;
      },
    },
    splitLine: { show: false },
  };
}

export function valueAxis(opts: { name?: string; max?: number; min?: number; unit?: string; position?: "left" | "right"; offset?: number; show?: boolean }) {
  const c = axisColors();
  return {
    type: "value",
    name: opts.name,
    nameTextStyle: { color: c.text, fontSize: 11 },
    min: opts.min ?? 0,
    max: opts.max,
    position: opts.position ?? "left",
    offset: opts.offset ?? 0,
    show: opts.show ?? true,
    axisLabel: { color: c.text, fontSize: 11, formatter: (v: number) => `${v}${opts.unit ?? ""}` },
    splitLine: { lineStyle: { color: c.grid } },
    axisLine: { show: false },
  };
}

export function lineSeries(name: string, color: string, data: [number, number | null][], yAxisIndex = 0, area = false) {
  return {
    name,
    type: "line",
    showSymbol: false,
    smooth: 0.25,
    sampling: "lttb",
    connectNulls: false,
    yAxisIndex,
    lineStyle: { width: 1.6, color },
    itemStyle: { color },
    areaStyle: area ? { color, opacity: 0.08 } : undefined,
    data,
  };
}

export function tooltipFormatter(units: Record<string, string>) {
  return (params: any[]) => {
    if (!params.length) return "";
    const t = fmtTime(params[0].value[0]);
    const rows = params
      .map((p: any) => {
        const v = p.value[1];
        const unit = units[p.seriesName] ?? "";
        const txt = v === null || v === undefined ? "无数据" : `${Number(v).toFixed(unit === "RPM" ? 0 : 1)} ${unit}`;
        return `<div style="display:flex;justify-content:space-between;gap:16px"><span>${p.marker}${p.seriesName}</span><b>${txt}</b></div>`;
      })
      .join("");
    return `<div style="margin-bottom:4px;opacity:.7">${t}</div>${rows}`;
  };
}

export function markerColor(level: string) {
  return level === "critical" ? "#ef4444" : level === "high" ? "#f97316" : "#eab308";
}
