// 全局状态：订阅后端实时快照，维护实时曲线数据、对话框与提示

import { markRaw, reactive } from "vue";
import { listen } from "@tauri-apps/api/event";
import { api } from "./api";
import type { KillMode, Marker, ProcessInfo, Settings, Snapshot, StaticInfo, SysPoint } from "./types";

/** 实时曲线最多保留 30 分钟 */
const KEEP_MS = 30 * 60 * 1000;

export interface Toast {
  id: number;
  kind: "success" | "error" | "info" | "warning";
  title: string;
  message?: string;
  action?: { label: string; run: () => void };
}

export interface Observation {
  name: string;
  pid: number;
  at: number;
  before: { cpu: number; cpuTemp: number | null; gpuTemp: number | null; fan: number | null };
  after: { cpu: number; cpuTemp: number | null; gpuTemp: number | null; fan: number | null } | null;
}

export const store = reactive({
  ready: false,
  snapshot: null as Snapshot | null,
  staticInfo: null as StaticInfo | null,
  settings: null as Settings | null,
  points: markRaw([] as SysPoint[]),
  markers: markRaw([] as Marker[]),
  /** 最近 120 次磁盘采样（仪表盘迷你曲线） */
  disk: markRaw([] as { ts: number; read: number | null; write: number | null; active: number | null }[]),
  /** 曲线数据版本号（points 为非响应式数组，通过版本号触发更新） */
  version: 0,
  paused: false,
  elevated: false,
  detail: null as { pid: number; createTime: string } | null,
  /** returnTo：从进程详情发起时记录详情窗口，取消操作后自动恢复 */
  kill: null as { proc: ProcessInfo; mode: KillMode; returnTo: { pid: number; createTime: string } | null } | null,
  toasts: [] as Toast[],
  observations: [] as Observation[],
  /** 已结束但快照尚未刷新的进程 key，用于立即从列表移除 */
  removed: new Set<string>(),
  navigateAction: null as string | null,
});

let toastId = 1;
export function toast(t: Omit<Toast, "id">, ms = 4500) {
  const id = toastId++;
  store.toasts.push({ ...t, id });
  setTimeout(() => dismissToast(id), ms);
}
export function dismissToast(id: number) {
  const i = store.toasts.findIndex((t) => t.id === id);
  if (i >= 0) store.toasts.splice(i, 1);
}

export function procKey(p: { pid: number; createTime: string }) {
  return `${p.pid}:${p.createTime}`;
}

export function openDetail(pid: number, createTime: string) {
  store.detail = { pid, createTime };
}

export function findProcess(pid: number, createTime?: string): ProcessInfo | undefined {
  return store.snapshot?.processes.find((p) => p.pid === pid && (createTime === undefined || p.createTime === createTime));
}

export function openKill(proc: ProcessInfo, mode: KillMode) {
  // 先关闭进程详情窗口，避免其遮挡结束进程确认框
  const returnTo = store.detail;
  store.detail = null;
  store.kill = { proc, mode, returnTo };
}

/** 结束进程后观察 CPU 和温度是否下降 */
export function startObservation(proc: ProcessInfo) {
  const s = store.snapshot;
  if (!s) return;
  const obs: Observation = {
    name: proc.name,
    pid: proc.pid,
    at: Date.now(),
    before: { cpu: s.cpu.usage, cpuTemp: s.cpu.temp, gpuTemp: s.gpuTemp, fan: s.fanRpm },
    after: null,
  };
  store.observations.unshift(obs);
  if (store.observations.length > 3) store.observations.pop();
  setTimeout(() => {
    const target = store.observations.find((o) => o === obs || (o.pid === obs.pid && o.at === obs.at));
    // 取最近 5 秒的平均值，避免瞬时波动
    const recent = store.points.filter((p) => p.ts >= Date.now() - 5000);
    const avg = (f: (p: SysPoint) => number | null) => {
      const vals = recent.map(f).filter((v): v is number => v !== null);
      return vals.length ? vals.reduce((a, b) => a + b, 0) / vals.length : null;
    };
    if (target) {
      target.after = {
        cpu: avg((p) => p.cpu) ?? store.snapshot?.cpu.usage ?? 0,
        cpuTemp: avg((p) => p.cpuTemp),
        gpuTemp: avg((p) => p.gpuTemp),
        fan: avg((p) => p.fan),
      };
    }
  }, 15000);
}

export function dismissObservation(o: Observation) {
  const i = store.observations.indexOf(o);
  if (i >= 0) store.observations.splice(i, 1);
}

function pushPoint(p: SysPoint) {
  const arr = store.points;
  if (arr.length && arr[arr.length - 1].ts >= p.ts) return;
  arr.push(p);
  const cutoff = p.ts - KEEP_MS;
  let drop = 0;
  while (drop < arr.length && arr[drop].ts < cutoff) drop++;
  if (drop) arr.splice(0, drop);
  store.version++;
}

export async function resync() {
  try {
    const rt = await api.realtime();
    store.points = markRaw(rt.points.filter((p) => p.ts >= Date.now() - KEEP_MS));
    store.markers = markRaw(rt.markers);
    store.version++;
    const snap = await api.snapshot();
    if (snap) store.snapshot = markRaw(snap);
    store.paused = await api.getPaused();
  } catch (e) {
    console.error(e);
  }
}

// ---------------------------------------------------------------------------
// 主题
// ---------------------------------------------------------------------------

const media = window.matchMedia("(prefers-color-scheme: dark)");
export function applyTheme(theme: string) {
  const dark = theme === "dark" || (theme === "system" && media.matches);
  document.documentElement.classList.toggle("dark", dark);
  window.dispatchEvent(new CustomEvent("fg-theme"));
}
media.addEventListener("change", () => {
  if (store.settings?.theme === "system") applyTheme("system");
});

export async function saveSettings(patch: Partial<Settings>, silent = true) {
  if (!store.settings) return;
  const next = { ...store.settings, ...patch } as Settings;
  try {
    const saved = await api.saveSettings(next);
    store.settings = saved;
    applyTheme(saved.theme);
    if (!silent) toast({ kind: "success", title: "设置已保存" });
  } catch (e) {
    toast({ kind: "error", title: "保存设置失败", message: String(e) });
  }
}

let pawnioBusy = false;
export async function installPawnio() {
  if (pawnioBusy) return;
  pawnioBusy = true;
  toast({ kind: "info", title: "正在安装 PawnIO 驱动", message: "请在 UAC 提示中点击「是」，安装完成后会自动重新加载传感器" });
  try {
    const v = await api.pawnioInstall();
    toast({
      kind: "success",
      title: `PawnIO ${v} 安装完成`,
      message: store.elevated ? "传感器服务已重新加载" : "以管理员身份运行 FanGuard 后即可读取 CPU 温度、功耗与主板风扇",
      action: store.elevated ? undefined : { label: "以管理员身份重启", run: () => api.relaunchAdmin() },
    }, 8000);
  } catch (e) {
    toast({ kind: "error", title: "PawnIO 安装未完成", message: String(e), action: { label: "打开官网", run: () => api.pawnioOpenSite() } }, 8000);
  } finally {
    pawnioBusy = false;
  }
}

export async function toggleWhitelist(name: string) {
  if (!store.settings) return;
  const add = !store.settings.whitelist.some((w) => w.toLowerCase() === name.toLowerCase());
  try {
    store.settings = await api.setWhitelist(name, add);
    toast({
      kind: "success",
      title: add ? "已添加到白名单" : "已从白名单移除",
      message: add ? `${name} 将不再触发告警，并在诊断排序中靠后（安全信息仍会显示）` : name,
    });
  } catch (e) {
    toast({ kind: "error", title: "操作失败", message: String(e) });
  }
}

export function isWhitelisted(name: string) {
  return !!store.settings?.whitelist.some((w) => w.toLowerCase() === name.toLowerCase());
}

// ---------------------------------------------------------------------------
// 初始化
// ---------------------------------------------------------------------------

export async function initStore(onNavigate: (path: string, action: string | null) => void) {
  const [info, settings, elevated] = await Promise.all([api.staticInfo(), api.settings(), api.isElevated()]);
  store.staticInfo = markRaw(info);
  store.settings = settings;
  store.elevated = elevated;
  applyTheme(settings.theme);
  await resync();
  store.ready = true;

  await listen<Snapshot>("snapshot", (e) => {
    const s = e.payload;
    // 清除已确认退出的进程
    if (store.removed.size) {
      const alive = new Set(s.processes.map(procKey));
      for (const k of store.removed) if (!alive.has(k)) store.removed.delete(k);
      s.processes = s.processes.filter((p) => !store.removed.has(procKey(p)));
    }
    store.snapshot = markRaw(s);
    pushPoint(s.point);
    store.disk.push({ ts: s.ts, read: s.disk.read, write: s.disk.write, active: s.disk.active });
    if (store.disk.length > 120) store.disk.splice(0, store.disk.length - 120);
    if (s.newMarkers.length) {
      store.markers.push(...s.newMarkers);
      store.version++;
    }
  });
  await listen("resync", () => resync());
  await listen<{ path: string; action: string | null }>("navigate", (e) => {
    onNavigate(e.payload.path, e.payload.action);
  });
  await listen<boolean>("paused-changed", (e) => {
    store.paused = e.payload;
  });
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") resync();
  });
}
