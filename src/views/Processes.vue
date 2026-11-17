<script setup lang="ts">
import { computed, onActivated, onMounted, ref, watch } from "vue";
import { useRoute } from "vue-router";
import { Search, RefreshCw, ArrowDown, ArrowUp, MoreHorizontal, ShieldCheck, TriangleAlert, Pause, Play } from "lucide-vue-next";
import Button from "../components/ui/Button.vue";
import Badge from "../components/ui/Badge.vue";
import Select from "../components/ui/Select.vue";
import Segmented from "../components/ui/Segmented.vue";
import Card from "../components/ui/Card.vue";
import Observations from "../components/Observations.vue";
import { store, openDetail, openKill, saveSettings } from "../store";
import { openMenu } from "../menu";
import { api } from "../api";
import { fmtBytes, fmtRate, fmtUptime, gradeColor } from "../format";
import type { ProcessInfo } from "../types";

defineOptions({ name: "Processes" });

const route = useRoute();
const query = ref("");
const filter = ref<"all" | "user" | "system" | "high">("all");
const tab = ref("list");
const sortKey = ref<keyof ProcessInfo | "disk">("cpu");
const sortDesc = ref(true);
const autoRefresh = ref(true);
const frozen = ref<ProcessInfo[] | null>(null);

function applyRoute() {
  if (route.query.filter === "high") {
    filter.value = "high";
    tab.value = "list";
  }
}
onMounted(applyRoute);
onActivated(applyRoute);
watch(() => route.query.filter, applyRoute);

watch(autoRefresh, (v) => {
  frozen.value = v ? null : [...(store.snapshot?.processes ?? [])];
});

function manualRefresh() {
  api.refreshNow();
  if (!autoRefresh.value) {
    setTimeout(() => (frozen.value = [...(store.snapshot?.processes ?? [])]), 300);
  }
}

const source = computed(() => frozen.value ?? store.snapshot?.processes ?? []);
const st = computed(() => store.settings!);

const isSystem = (p: ProcessInfo) => p.kind !== "user";

const hiddenBySettings = ref(0);
const rows = computed(() => {
  let list = source.value.filter((p) => !store.removed.has(`${p.pid}:${p.createTime}`));
  let hidden = 0;
  if (filter.value === "all") {
    const before = list.length;
    list = list.filter((p) => {
      if (!st.value.showServices && p.kind === "service") return false;
      if (!st.value.showSystemProcesses && (p.kind === "critical" || p.kind === "system")) return false;
      return true;
    });
    hidden = before - list.length;
  } else if (filter.value === "user") list = list.filter((p) => !isSystem(p));
  else if (filter.value === "system") list = list.filter(isSystem);
  else if (filter.value === "high")
    list = list.filter((p) => p.cpu >= 5 || (p.gpu ?? 0) >= 10 || p.score >= 40 || p.memory >= 1024 ** 3);
  hiddenBySettings.value = hidden;

  const q = query.value.trim().toLowerCase();
  if (q) {
    const isNum = /^\d+$/.test(q);
    list = list.filter((p) => p.name.toLowerCase().includes(q) || (isNum && String(p.pid).startsWith(q)));
  }
  const k = sortKey.value;
  const dir = sortDesc.value ? -1 : 1;
  return [...list].sort((a, b) => {
    let va: any;
    let vb: any;
    if (k === "disk") {
      va = a.ioRead + a.ioWrite;
      vb = b.ioRead + b.ioWrite;
    } else if (k === "startTime") {
      // 运行时间：启动越早运行越久
      va = -a.startTime;
      vb = -b.startTime;
    } else {
      va = (a as any)[k] ?? -1;
      vb = (b as any)[k] ?? -1;
    }
    if (typeof va === "string") return va.localeCompare(vb) * dir;
    if (va === vb) return a.pid - b.pid;
    return (va - vb) * dir;
  });
});

function sortBy(k: keyof ProcessInfo | "disk") {
  if (sortKey.value === k) sortDesc.value = !sortDesc.value;
  else {
    sortKey.value = k;
    sortDesc.value = k !== "name" && k !== "pid";
  }
}

// ---------- 虚拟滚动 ----------
const ROW_H = 34;
const scroller = ref<HTMLDivElement>();
const scrollTop = ref(0);
const viewH = ref(600);
function onScroll() {
  scrollTop.value = scroller.value?.scrollTop ?? 0;
  viewH.value = scroller.value?.clientHeight ?? 600;
}
onMounted(() => {
  onScroll();
  new ResizeObserver(onScroll).observe(scroller.value!);
});
const visible = computed(() => {
  const start = Math.max(0, Math.floor(scrollTop.value / ROW_H) - 8);
  const end = Math.min(rows.value.length, Math.ceil((scrollTop.value + viewH.value) / ROW_H) + 8);
  return { start, items: rows.value.slice(start, end) };
});

const intervalOptions = [
  { label: "0.5 秒", value: 500 },
  { label: "1 秒", value: 1000 },
  { label: "2 秒", value: 2000 },
  { label: "5 秒", value: 5000 },
];
const filterOptions = [
  { label: "全部进程", value: "all" },
  { label: "用户进程", value: "user" },
  { label: "系统进程", value: "system" },
  { label: "仅高负载进程", value: "high" },
];

const columns: { key: keyof ProcessInfo | "disk"; label: string; align?: string }[] = [
  { key: "name", label: "进程名称" },
  { key: "pid", label: "PID", align: "right" },
  { key: "cpu", label: "CPU", align: "right" },
  { key: "memory", label: "内存", align: "right" },
  { key: "gpu", label: "GPU", align: "right" },
  { key: "gpuMem", label: "显存", align: "right" },
  { key: "disk", label: "磁盘 I/O", align: "right" },
  { key: "startTime", label: "运行时间", align: "right" },
  { key: "score", label: "状态" },
];
const gridCols = "minmax(200px,1fr) 70px 72px 90px 64px 84px 96px 110px 120px 92px";

const now = computed(() => store.snapshot?.ts ?? Date.now());

function cpuColor(v: number) {
  if (v >= 50) return "color-mix(in srgb, var(--danger) 22%, transparent)";
  if (v >= 20) return "color-mix(in srgb, var(--warn) 18%, transparent)";
  if (v >= 5) return "color-mix(in srgb, var(--notice) 14%, transparent)";
  return "transparent";
}

// ---------- 排行 ----------
const all = computed(() => (store.snapshot?.processes ?? []).filter((p) => !store.removed.has(`${p.pid}:${p.createTime}`)));
const topCpu = computed(() => [...all.value].sort((a, b) => b.cpu - a.cpu).slice(0, 10));
const topGpu = computed(() => [...all.value].filter((p) => p.gpu !== null).sort((a, b) => (b.gpu ?? 0) - (a.gpu ?? 0)).slice(0, 10));
const topMem = computed(() => [...all.value].sort((a, b) => b.memory - a.memory).slice(0, 10));
</script>

<template>
  <div class="flex h-full flex-col">
    <div class="space-y-3 border-b border-border px-5 pt-4 pb-3">
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-4">
          <h1 class="text-lg font-semibold">进程管理</h1>
          <Segmented
            v-model="tab"
            :options="[
              { label: '全部进程', value: 'list' },
              { label: '高负载排行', value: 'rank' },
            ]"
          />
        </div>
        <div class="flex items-center gap-5 text-[13px] tabular">
          <span class="text-muted-foreground">CPU：<b class="text-foreground">{{ store.snapshot?.cpu.usage.toFixed(0) }}%</b></span>
          <span class="text-muted-foreground">内存：<b class="text-foreground">{{ store.snapshot?.memory.usage.toFixed(0) }}%</b></span>
          <span class="text-muted-foreground"
            >GPU：<b class="text-foreground">{{ store.snapshot?.gpuUsage == null ? "不可用" : store.snapshot.gpuUsage.toFixed(0) + "%" }}</b></span
          >
          <span class="text-muted-foreground">进程：<b class="text-foreground">{{ store.snapshot?.processCount }}</b></span>
        </div>
      </div>
      <div v-if="tab === 'list'" class="flex items-center gap-2">
        <div class="relative w-[280px]">
          <Search class="absolute top-1/2 left-2.5 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
          <input
            v-model="query"
            placeholder="搜索进程名称或 PID..."
            class="h-8 w-full rounded-md border border-border bg-card pr-2 pl-8 text-[13px] outline-none focus:ring-2 focus:ring-primary/40"
          />
        </div>
        <Select v-model="filter" :options="filterOptions" />
        <Button variant="outline" @click="manualRefresh"><RefreshCw class="h-3.5 w-3.5" /> 刷新</Button>
        <Button variant="ghost" @click="autoRefresh = !autoRefresh">
          <component :is="autoRefresh ? Pause : Play" class="h-3.5 w-3.5" />
          {{ autoRefresh ? "暂停自动刷新" : "恢复自动刷新" }}
        </Button>
        <div class="ml-auto flex items-center gap-2 text-xs text-muted-foreground">
          刷新频率
          <Select
            size="sm"
            :model-value="st.processIntervalMs"
            :options="intervalOptions"
            @update:model-value="(v: number) => saveSettings({ processIntervalMs: v })"
          />
        </div>
      </div>
    </div>

    <div class="px-5 pt-3 empty:hidden"><Observations /></div>

    <!-- 进程列表 -->
    <template v-if="tab === 'list'">
      <div class="flex items-center justify-between px-5 py-1.5 text-[11px] text-muted-foreground">
        <span>
          显示 {{ rows.length }} 个进程
          <template v-if="hiddenBySettings"> · 已按设置隐藏 {{ hiddenBySettings }} 个系统进程/服务</template>
          <template v-if="!autoRefresh"> · <span class="text-warn">自动刷新已暂停</span></template>
        </span>
        <span>CPU 为整机百分比（已按 {{ store.staticInfo?.threads }} 个逻辑处理器归一化）· 右键进程可打开操作菜单</span>
      </div>
      <div class="mx-5 mb-4 flex min-h-0 flex-1 flex-col overflow-hidden rounded-lg border border-border bg-card">
        <div class="grid shrink-0 border-b border-border text-xs text-muted-foreground" :style="{ gridTemplateColumns: gridCols }">
          <button
            v-for="c in columns"
            :key="c.key"
            class="flex h-9 items-center gap-1 px-3 font-medium hover:text-foreground"
            :class="c.align === 'right' ? 'justify-end' : ''"
            @click="sortBy(c.key)"
          >
            {{ c.label }}
            <component :is="sortDesc ? ArrowDown : ArrowUp" v-if="sortKey === c.key" class="h-3 w-3 text-primary" />
          </button>
          <div class="flex h-9 items-center justify-center px-3 font-medium">操作</div>
        </div>
        <div ref="scroller" class="min-h-0 flex-1 overflow-auto" @scroll="onScroll">
          <div :style="{ height: `${rows.length * ROW_H}px`, position: 'relative' }">
            <div
              v-for="(p, i) in visible.items"
              :key="p.pid + ':' + p.createTime"
              class="absolute right-0 left-0 grid items-center border-b border-border/40 text-[13px] hover:bg-accent/60"
              :style="{ top: `${(visible.start + i) * ROW_H}px`, height: `${ROW_H}px`, gridTemplateColumns: gridCols }"
              @dblclick="openDetail(p.pid, p.createTime)"
              @click="openDetail(p.pid, p.createTime)"
              @contextmenu="openMenu(p, $event)"
            >
              <div class="flex min-w-0 items-center gap-2 px-3">
                <ShieldCheck v-if="p.kind === 'critical'" class="h-3.5 w-3.5 shrink-0 text-ok" />
                <TriangleAlert v-else-if="p.status === 'abnormal'" class="h-3.5 w-3.5 shrink-0" :style="{ color: gradeColor(p.score) }" />
                <span v-else class="w-3.5 shrink-0" />
                <span class="truncate" :title="p.path ?? p.name">{{ p.name }}</span>
                <Badge v-if="p.whitelisted" color="var(--primary)">白名单</Badge>
              </div>
              <div class="px-3 text-right text-muted-foreground tabular">{{ p.pid }}</div>
              <div class="h-full px-3 text-right leading-[34px] tabular" :style="{ background: cpuColor(p.cpu) }">{{ p.cpu.toFixed(1) }}%</div>
              <div class="px-3 text-right tabular">{{ fmtBytes(p.memory) }}</div>
              <div class="px-3 text-right tabular">{{ p.gpu === null ? "-" : p.gpu.toFixed(0) + "%" }}</div>
              <div class="px-3 text-right tabular">{{ p.gpuMem === null ? "-" : p.gpuMem > 0 ? fmtBytes(p.gpuMem) : "0" }}</div>
              <div class="px-3 text-right text-xs tabular">{{ p.ioRead + p.ioWrite < 1 ? "0" : fmtRate(p.ioRead + p.ioWrite) }}</div>
              <div class="px-3 text-right text-xs text-muted-foreground tabular">{{ fmtUptime(p.startTime, now) }}</div>
              <div class="px-3">
                <Badge v-if="p.kind === 'critical'" color="var(--ok)">受保护</Badge>
                <Badge v-else-if="p.status === 'abnormal'" :color="gradeColor(p.score)">异常 · {{ p.score.toFixed(0) }}</Badge>
                <Badge v-else-if="p.score >= 40" color="var(--notice)">关注 · {{ p.score.toFixed(0) }}</Badge>
                <span v-else class="text-xs text-muted-foreground">正常</span>
              </div>
              <div class="flex items-center justify-center gap-1 px-2" @click.stop @dblclick.stop>
                <Badge v-if="p.kind === 'critical'" color="var(--muted-foreground)">保护</Badge>
                <Button v-else size="xs" variant="secondary" @click="openKill(p, 'graceful')">结束</Button>
                <button class="flex h-6 w-6 items-center justify-center rounded hover:bg-accent" @click="openMenu(p, $event)">
                  <MoreHorizontal class="h-3.5 w-3.5" />
                </button>
              </div>
            </div>
          </div>
          <div v-if="!rows.length" class="py-16 text-center text-muted-foreground">没有匹配的进程</div>
        </div>
      </div>
    </template>

    <!-- 高负载排行 -->
    <div v-else class="min-h-0 flex-1 overflow-auto p-5">
      <div class="grid grid-cols-3 gap-4">
        <Card
          v-for="board in [
            { title: 'CPU 高负载排行', list: topCpu, val: (p: ProcessInfo) => p.cpu.toFixed(1) + '%', bar: (p: ProcessInfo) => p.cpu },
            {
              title: 'GPU 高负载排行',
              list: topGpu,
              val: (p: ProcessInfo) => (p.gpu ?? 0).toFixed(1) + '%',
              bar: (p: ProcessInfo) => p.gpu ?? 0,
            },
            {
              title: '内存占用排行',
              list: topMem,
              val: (p: ProcessInfo) => fmtBytes(p.memory),
              bar: (p: ProcessInfo) => (p.memory / (store.staticInfo?.totalMemory || 1)) * 100,
            },
          ]"
          :key="board.title"
          :title="board.title"
          subtitle="前 10 个进程，点击查看详情"
        >
          <div v-if="board.title.startsWith('GPU') && !store.snapshot?.gpuAvailable" class="py-8 text-center text-xs text-muted-foreground">
            当前系统不支持 GPU 性能计数器
          </div>
          <div v-else class="space-y-0.5">
            <button
              v-for="(p, i) in board.list"
              :key="p.pid + ':' + p.createTime"
              class="flex w-full items-center gap-2 rounded px-1.5 py-1.5 text-left hover:bg-accent"
              @click="openDetail(p.pid, p.createTime)"
              @contextmenu="openMenu(p, $event)"
            >
              <span class="w-4 text-xs text-muted-foreground tabular">{{ i + 1 }}</span>
              <div class="min-w-0 flex-1">
                <div class="flex items-center justify-between gap-2">
                  <span class="truncate text-[13px]">{{ p.name }}</span>
                  <span class="text-[13px] font-medium tabular">{{ board.val(p) }}</span>
                </div>
                <div class="mt-1 h-1 overflow-hidden rounded-full bg-muted">
                  <div class="h-full rounded-full bg-primary" :style="{ width: Math.min(100, board.bar(p)) + '%' }" />
                </div>
              </div>
            </button>
          </div>
        </Card>
      </div>
    </div>
  </div>
</template>
