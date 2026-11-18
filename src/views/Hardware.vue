<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useRoute } from "vue-router";
import { ShieldAlert, ChevronRight, ChevronDown, CircleCheck, CircleAlert, HardDrive } from "lucide-vue-next";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Badge from "../components/ui/Badge.vue";
import Progress from "../components/ui/Progress.vue";
import RealtimeChart from "../components/RealtimeChart.vue";
import { api } from "../api";
import { store, toast, saveSettings, installPawnio } from "../store";
import Segmented from "../components/ui/Segmented.vue";
import Switch from "../components/ui/Switch.vue";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { fmtBytes, fmtMhz, fmtPct, fmtRate, fmtRpm, fmtTemp, fmtWatt, statusColor, tempStatus, loadStatus, fanStatus } from "../format";
import type { FanReading, HwNode, Settings } from "../types";

const route = useRoute();
const s = computed(() => store.snapshot);
const info = computed(() => store.staticInfo);
const st = computed(() => store.settings!);
const raw = ref<HwNode[]>([]);
const showRaw = ref(false);
const expanded = ref<Set<string>>(new Set());
let timer: number | undefined;

async function loadRaw() {
  try {
    raw.value = await api.sensorRaw();
  } catch {
    /* ignore */
  }
}

watch(showRaw, (v) => {
  clearInterval(timer);
  if (v) {
    loadRaw();
    timer = window.setInterval(loadRaw, 2000);
  }
});

onMounted(async () => {
  await nextTick();
  if (route.hash) document.querySelector(route.hash)?.scrollIntoView({ behavior: "smooth" });
});
watch(
  () => route.hash,
  (h) => h && document.querySelector(h)?.scrollIntoView({ behavior: "smooth" }),
);
onBeforeUnmount(() => clearInterval(timer));

const sensorState = computed(() => {
  const ss = s.value?.sensor;
  if (!ss) return { ok: false, text: "未知" };
  const map: Record<string, string> = {
    running: "运行中",
    starting: "正在启动",
    notFound: "未找到传感器服务",
    exited: "已退出（正在重启）",
    paused: "已暂停",
    disabled: "已停用（仅使用 HWiNFO）",
  };
  return { ok: ss.status === "running", text: map[ss.status] ?? ss.status };
});

const hwinfoState = computed(() => {
  const ss = s.value?.sensor;
  const map: Record<string, string> = {
    running: "已连接",
    notFound: "未检测到",
    inactive: "共享内存已关闭",
    invalid: "格式无法识别",
    stale: "数据已过期",
    disabled: "未启用",
  };
  return { ok: ss?.hwinfo === "running", text: map[ss?.hwinfo ?? "notFound"] ?? ss?.hwinfo };
});

const anyOk = computed(() => sensorState.value.ok || hwinfoState.value.ok);

const mgr = computed(() => s.value!.hwinfoMgr);
const mgrStageText = computed(
  () =>
    ({
      disabled: "已关闭",
      notInstalled: "未找到 HWiNFO",
      importing: "正在导入",
      ready: "已就绪",
      launching: "正在启动",
      running: "运行中",
      declined: "未授权",
      error: "出错",
    })[mgr.value.stage] ?? mgr.value.stage,
);
const mgrColor = computed(() =>
  mgr.value.stage === "running" && s.value!.sensor.hwinfo === "running"
    ? "var(--ok)"
    : mgr.value.stage === "error" || mgr.value.stage === "declined"
      ? "var(--danger)"
      : "var(--notice)",
);
const importing = ref(false);

function launchHwinfo() {
  api.hwinfoLaunch();
  toast({ kind: "info", title: "正在启动 HWiNFO", message: store.elevated ? undefined : "HWiNFO 需要管理员权限，请在 UAC 提示中点击「是」" });
}

async function importHwinfo() {
  const file = await openDialog({
    title: "选择 HWiNFO 便携版压缩包或 HWiNFO64.exe",
    multiple: false,
    filters: [{ name: "HWiNFO", extensions: ["zip", "exe"] }],
  });
  if (!file || Array.isArray(file)) return;
  importing.value = true;
  try {
    await api.hwinfoImport(file);
    toast({ kind: "success", title: "HWiNFO 已导入", message: "已完成配置，正在启动" });
  } catch (e) {
    toast({ kind: "error", title: "导入失败", message: String(e) });
  } finally {
    importing.value = false;
  }
}

function setSource(v: string) {
  saveSettings({ sensorSource: v as Settings["sensorSource"] });
}

async function relaunch() {
  try {
    await api.relaunchAdmin();
  } catch (e) {
    toast({ kind: "error", title: "未能以管理员身份重启", message: String(e) });
  }
}

function toggle(id: string) {
  const n = new Set(expanded.value);
  n.has(id) ? n.delete(id) : n.add(id);
  expanded.value = n;
}

const unit: Record<string, string> = {
  Temperature: "°C",
  Fan: " RPM",
  Power: " W",
  Clock: " MHz",
  Load: "%",
  Voltage: " V",
  Current: " A",
  Control: "%",
  Data: " GB",
  SmallData: " MB",
  Throughput: " B/s",
  Level: "%",
  Factor: "",
  Frequency: " Hz",
  Energy: " mWh",
  Noise: " dBA",
  TimeSpan: " s",
  Flow: " L/h",
};
const typeName: Record<string, string> = {
  Temperature: "温度",
  Fan: "风扇",
  Power: "功耗",
  Clock: "频率",
  Load: "负载",
  Voltage: "电压",
  Current: "电流",
  Control: "风扇控制",
  Data: "数据",
  SmallData: "数据",
  Throughput: "吞吐",
  Level: "等级",
  Factor: "系数",
};

const fanKind: Record<string, string> = { cpu: "CPU 风扇", gpu: "GPU 风扇", case: "机箱风扇" };
const fansByKind = computed(() => {
  const groups: Record<string, FanReading[]> = { cpu: [], gpu: [], case: [] };
  for (const f of s.value?.fans ?? []) groups[f.kind].push(f);
  return groups;
});
</script>

<template>
  <div v-if="s && info" class="h-full overflow-auto">
    <div class="mx-auto max-w-[1400px] space-y-4 p-5">
      <div class="flex items-center justify-between">
        <div>
          <h1 class="text-lg font-semibold">硬件监控</h1>
          <div class="text-xs text-muted-foreground">{{ info.hostname }} · {{ info.osName }} {{ info.osBuild }}</div>
        </div>
      </div>

      <!-- 传感器状态 -->
      <div class="rounded-lg border border-border bg-card px-4 py-3 text-xs">
        <div class="flex items-start gap-3">
          <component :is="anyOk ? CircleCheck : CircleAlert" class="mt-0.5 h-4 w-4 shrink-0" :class="anyOk ? 'text-ok' : 'text-warn'" />
          <div class="min-w-0 flex-1 space-y-1">
            <div>
              当前数据来源：<b>{{ s.sensor.source === "none" ? "无" : s.sensor.source }}</b>
              <span v-if="s.sensor.nvml" class="ml-2 text-muted-foreground">· NVIDIA NVML 已启用</span>
            </div>
            <div class="flex flex-wrap gap-x-5 text-muted-foreground">
              <span>LibreHardwareMonitor {{ s.sensor.lhmVersion ?? "" }}：<b class="text-foreground">{{ sensorState.text }}</b></span>
              <span>
                HWiNFO 共享内存：<b :class="hwinfoState.ok ? 'text-ok' : 'text-foreground'">{{ hwinfoState.text }}</b>
                <template v-if="hwinfoState.ok">（{{ s.sensor.hwinfoSensors }} 个读数）</template>
              </span>
            </div>
            <div v-if="s.sensor.message" class="text-muted-foreground">{{ s.sensor.message }}</div>
          </div>
          <div class="flex shrink-0 items-center gap-2">
            <Segmented
              :model-value="st.sensorSource"
              :options="[
                { label: '自动', value: 'auto' },
                { label: 'HWiNFO', value: 'hwinfo' },
                { label: 'LibreHardwareMonitor', value: 'lhm' },
              ]"
              @update:model-value="setSource"
            />
            <Button v-if="!store.elevated" size="sm" variant="outline" @click="relaunch">
              <ShieldAlert class="h-3.5 w-3.5" /> 以管理员身份重启
            </Button>
          </div>
        </div>
        <!-- PawnIO 驱动 -->
        <div
          v-if="st.sensorSource !== 'hwinfo' && (!s.pawnio.installed || s.pawnio.updateAvailable)"
          class="mt-2.5 flex items-center gap-3 rounded-md border border-primary/40 bg-primary/10 p-2.5"
        >
          <HardDrive class="h-4 w-4 shrink-0 text-primary" />
          <div class="min-w-0 flex-1">
            <div class="font-medium">
              <template v-if="!s.pawnio.installed">未安装 PawnIO 驱动</template>
              <template v-else>PawnIO 驱动可更新（已安装 {{ s.pawnio.version }}，内置 {{ s.pawnio.bundledVersion }}）</template>
            </div>
            <div class="text-muted-foreground">
              LibreHardwareMonitor 通过 PawnIO 读取 CPU 温度 / 功耗以及台式机主板的风扇转速，未安装时即使以管理员身份运行也读不到。
              PawnIO 是开源、已签名的驱动，安装只需一次 UAC 授权{{ s.pawnio.bundledVersion ? "（安装程序已内置，无需联网）" : "（将从官方 GitHub 下载）" }}。
            </div>
          </div>
          <div class="flex shrink-0 items-center gap-2">
            <Button size="sm" variant="ghost" @click="api.pawnioOpenSite()">了解 PawnIO</Button>
            <Button size="sm" :loading="s.pawnio.installing" @click="installPawnio">{{ s.pawnio.installed ? "更新驱动" : "安装驱动" }}</Button>
          </div>
        </div>
        <div v-else-if="st.sensorSource !== 'hwinfo' && s.pawnio.installed && !store.elevated" class="mt-2.5 text-muted-foreground">
          PawnIO {{ s.pawnio.version }} 已安装，以管理员身份运行 FanGuard 后 LibreHardwareMonitor 即可使用它读取 CPU 温度与主板风扇。
        </div>

        <!-- 同方 / Uniwill 笔记本 EC -->
        <div v-if="s.sensor.ec === 'needAdmin' && !store.elevated" class="mt-2.5 flex items-center gap-3 rounded-md border border-warn/40 bg-warn/10 p-2.5">
          <ShieldAlert class="h-4 w-4 shrink-0 text-warn" />
          <div class="min-w-0 flex-1">
            <div class="font-medium">检测到同方 / Uniwill 模具笔记本的嵌入式控制器（EC）</div>
            <div class="text-muted-foreground">
              这类笔记本的风扇由 EC 控制，转速只能通过厂商 WMI 接口读取，Windows 仅允许管理员访问。以管理员身份运行后即可显示 CPU / GPU 风扇转速与占空比（只读取，不会修改风扇设置）。
            </div>
          </div>
          <div class="flex shrink-0 items-center gap-2">
            <span class="text-muted-foreground">始终以管理员运行</span>
            <Switch :model-value="st.runAsAdmin" @update:model-value="(v) => saveSettings({ runAsAdmin: v })" />
            <Button size="sm" @click="relaunch">以管理员身份重启</Button>
          </div>
        </div>
        <div v-else-if="s.sensor.ec === 'available'" class="mt-2.5 text-muted-foreground">
          已通过嵌入式控制器（Uniwill EC）读取笔记本风扇转速。
        </div>

        <!-- HWiNFO 自动集成 -->
        <div v-if="st.sensorSource !== 'lhm'" class="mt-2.5 rounded-md bg-muted p-2.5">
          <div class="flex items-center gap-3">
            <div class="min-w-0 flex-1">
              <div class="flex items-center gap-2">
                <span class="font-medium">HWiNFO 自动集成</span>
                <Badge :color="mgrColor">{{ mgrStageText }}</Badge>
                <span v-if="mgr.bundled" class="text-muted-foreground">· FanGuard 内置</span>
                <span v-else-if="mgr.managed" class="text-muted-foreground">· 托管副本</span>
              </div>
              <div v-if="mgr.path" class="mt-0.5 truncate text-muted-foreground" :title="mgr.path">{{ mgr.path }}</div>
              <div v-if="mgr.message" class="mt-0.5 text-muted-foreground">{{ mgr.message }}</div>
            </div>
            <div class="flex shrink-0 items-center gap-2">
              <span class="text-muted-foreground">自动</span>
              <Switch :model-value="st.hwinfoAuto" @update:model-value="(v) => saveSettings({ hwinfoAuto: v })" />
              <Button v-if="mgr.path && !mgr.running" size="sm" variant="secondary" @click="launchHwinfo">启动 HWiNFO</Button>
              <template v-if="!mgr.path">
                <Button size="sm" variant="secondary" @click="api.hwinfoOpenDownload()">下载 HWiNFO</Button>
                <Button size="sm" variant="outline" :loading="importing" @click="importHwinfo">导入文件…</Button>
              </template>
            </div>
          </div>
          <div v-if="!mgr.path" class="mt-2 text-muted-foreground">
            当前版本的 FanGuard 未内置 HWiNFO（构建时未找到 hwinfo.zip）。可将 HWiNFO 便携版（hwi_xxx.zip）放到「下载」文件夹或 FanGuard 所在目录，FanGuard 会自动校验签名并导入；也可点击「导入文件…」。
          </div>
        </div>
      </div>

      <!-- CPU -->
      <Card id="cpu" title="CPU" :subtitle="info.cpuModel">
        <div class="grid grid-cols-[1fr_1.2fr] gap-6">
          <div class="grid grid-cols-2 gap-x-6 gap-y-2.5 text-[13px]">
            <div class="flex justify-between"><span class="text-muted-foreground">使用率</span><b class="tabular">{{ fmtPct(s.cpu.usage, 1) }}</b></div>
            <div class="flex justify-between">
              <span class="text-muted-foreground">温度</span>
              <b class="tabular" :style="{ color: statusColor[tempStatus(s.cpu.temp, st.cpuTempThreshold)] }">{{ fmtTemp(s.cpu.temp) }}</b>
            </div>
            <div class="flex justify-between"><span class="text-muted-foreground">核心数量</span><b>{{ info.cores }}</b></div>
            <div class="flex justify-between"><span class="text-muted-foreground">线程数量</span><b>{{ info.threads }}</b></div>
            <div class="flex justify-between">
              <span class="text-muted-foreground">当前频率</span>
              <b class="tabular">{{ fmtMhz(s.cpu.clock) }}<span v-if="s.cpu.clockSource === 'estimated'" class="ml-1 text-[11px] font-normal text-muted-foreground">(估算)</span></b>
            </div>
            <div class="flex justify-between"><span class="text-muted-foreground">基准频率</span><b class="tabular">{{ fmtMhz(info.baseMhz) }}</b></div>
            <div class="flex justify-between"><span class="text-muted-foreground">功耗</span><b class="tabular">{{ fmtWatt(s.cpu.power) }}</b></div>
            <div class="flex justify-between"><span class="text-muted-foreground">厂商</span><b>{{ info.cpuVendor || "-" }}</b></div>
            <div class="flex justify-between"><span class="text-muted-foreground">进程 / 线程</span><b class="tabular">{{ s.processCount }} / {{ s.threadCount }}</b></div>
            <div class="flex justify-between"><span class="text-muted-foreground">句柄</span><b class="tabular">{{ s.handleCount.toLocaleString() }}</b></div>
          </div>
          <div>
            <div class="mb-1.5 text-xs text-muted-foreground">每核心使用率（逻辑处理器）</div>
            <div class="grid gap-1.5" :style="{ gridTemplateColumns: `repeat(${Math.min(8, Math.max(4, Math.ceil(s.cpu.perCore.length / 4)))}, minmax(0, 1fr))` }">
              <div v-for="(c, i) in s.cpu.perCore" :key="i" class="rounded bg-muted p-1.5">
                <div class="flex justify-between text-[10px] text-muted-foreground"><span>#{{ i }}</span><span class="tabular">{{ c.toFixed(0) }}%</span></div>
                <div class="mt-1 h-6 overflow-hidden rounded-sm bg-background">
                  <div class="w-full rounded-sm bg-primary transition-all" :style="{ height: '100%', transform: `translateY(${100 - c}%)`, opacity: 0.4 + c / 160 }" />
                </div>
              </div>
            </div>
          </div>
        </div>
        <div class="mt-4">
          <RealtimeChart :initial="['cpu', 'cpuTemp']" :only="['cpu', 'cpuTemp']" height="180px" />
        </div>
      </Card>

      <!-- GPU -->
      <Card id="gpu" title="GPU" :subtitle="`${s.gpus.length} 个显示适配器 · 支持 NVIDIA / AMD / Intel 核显`">
        <div v-if="!s.gpus.length" class="py-6 text-center text-muted-foreground">未检测到显卡</div>
        <div class="grid grid-cols-2 gap-4">
          <div v-for="g in s.gpus" :key="g.luid + g.name" class="rounded-md border border-border p-3">
            <div class="mb-2 flex items-center gap-2">
              <span class="font-medium">{{ g.name }}</span>
              <Badge>{{ g.vendor }}</Badge>
              <Badge v-if="g.integrated" color="var(--primary)">核显</Badge>
            </div>
            <div class="grid grid-cols-2 gap-x-6 gap-y-1.5 text-xs">
              <div class="flex justify-between"><span class="text-muted-foreground">使用率</span><b class="tabular">{{ g.usage === null ? "暂不支持读取" : fmtPct(g.usage, 1) }}</b></div>
              <div class="flex justify-between">
                <span class="text-muted-foreground">温度</span>
                <b class="tabular" :style="{ color: statusColor[tempStatus(g.temp, st.gpuTempThreshold)] }">{{ fmtTemp(g.temp) }}</b>
              </div>
              <div class="flex justify-between"><span class="text-muted-foreground">热点温度</span><b class="tabular">{{ g.hotspot === null ? "-" : fmtTemp(g.hotspot) }}</b></div>
              <div class="flex justify-between"><span class="text-muted-foreground">功耗</span><b class="tabular">{{ fmtWatt(g.power) }}</b></div>
              <div class="flex justify-between"><span class="text-muted-foreground">核心频率</span><b class="tabular">{{ fmtMhz(g.clock) }}</b></div>
              <div class="flex justify-between"><span class="text-muted-foreground">显存频率</span><b class="tabular">{{ fmtMhz(g.memClock) }}</b></div>
              <div class="flex justify-between"><span class="text-muted-foreground">显存容量</span><b class="tabular">{{ fmtBytes(g.memTotal) }}</b></div>
              <div class="flex justify-between"><span class="text-muted-foreground">显存使用</span><b class="tabular">{{ fmtBytes(g.memUsed) }} <template v-if="g.memUsage !== null">({{ g.memUsage.toFixed(0) }}%)</template></b></div>
              <div class="flex justify-between"><span class="text-muted-foreground">共享显存</span><b class="tabular">{{ fmtBytes(g.sharedUsed) }}</b></div>
              <div class="flex justify-between">
                <span class="text-muted-foreground">风扇</span>
                <b class="tabular">{{ g.fanRpm !== null ? fmtRpm(g.fanRpm) : g.fanPercent !== null ? `${g.fanPercent.toFixed(0)}%（占空比）` : "暂不支持读取" }}</b>
              </div>
            </div>
            <div class="mt-2"><Progress :value="g.usage" :color="statusColor[loadStatus(g.usage, st.gpuThreshold)]" /></div>
            <div v-if="g.engines.length" class="mt-2 flex flex-wrap gap-x-4 gap-y-0.5 text-[11px] text-muted-foreground">
              <span v-for="e in g.engines.slice(0, 6)" :key="e[0]">{{ e[0] }} {{ e[1].toFixed(0) }}%</span>
            </div>
          </div>
        </div>
        <div class="mt-4">
          <RealtimeChart :initial="['gpu', 'gpuTemp']" :only="['gpu', 'gpuTemp']" height="180px" />
        </div>
      </Card>

      <div class="grid grid-cols-2 gap-4">
        <!-- 内存 -->
        <Card id="memory" title="内存">
          <div class="mb-3"><Progress :value="s.memory.usage" :color="statusColor[loadStatus(s.memory.usage, 90)]" height="8px" /></div>
          <div class="grid grid-cols-2 gap-x-6 gap-y-2 text-[13px]">
            <div class="flex justify-between"><span class="text-muted-foreground">总内存</span><b class="tabular">{{ fmtBytes(s.memory.total) }}</b></div>
            <div class="flex justify-between"><span class="text-muted-foreground">内存使用率</span><b class="tabular">{{ fmtPct(s.memory.usage, 1) }}</b></div>
            <div class="flex justify-between"><span class="text-muted-foreground">已使用</span><b class="tabular">{{ fmtBytes(s.memory.used) }}</b></div>
            <div class="flex justify-between"><span class="text-muted-foreground">可用</span><b class="tabular">{{ fmtBytes(s.memory.available) }}</b></div>
            <div class="flex justify-between"><span class="text-muted-foreground">已提交</span><b class="tabular">{{ fmtBytes(s.memory.commitTotal) }} / {{ fmtBytes(s.memory.commitLimit) }}</b></div>
            <div class="flex justify-between"><span class="text-muted-foreground">页面文件使用</span><b class="tabular">{{ s.memory.pagefileUsage === null ? "-" : fmtPct(s.memory.pagefileUsage, 1) }}</b></div>
            <div class="flex justify-between"><span class="text-muted-foreground">系统缓存</span><b class="tabular">{{ fmtBytes(s.memory.cached) }}</b></div>
          </div>
        </Card>

        <!-- 磁盘 -->
        <Card id="disk" title="磁盘" subtitle="全部物理磁盘合计">
          <div class="mb-3"><Progress :value="s.disk.active" :color="statusColor[loadStatus(s.disk.active, 90)]" height="8px" /></div>
          <div class="grid grid-cols-2 gap-x-6 gap-y-2 text-[13px]">
            <div class="flex justify-between"><span class="text-muted-foreground">读取速度</span><b class="tabular">{{ fmtRate(s.disk.read) }}</b></div>
            <div class="flex justify-between"><span class="text-muted-foreground">写入速度</span><b class="tabular">{{ fmtRate(s.disk.write) }}</b></div>
            <div class="flex justify-between"><span class="text-muted-foreground">活动时间</span><b class="tabular">{{ fmtPct(s.disk.active, 1) }}</b></div>
            <div class="flex justify-between">
              <span class="text-muted-foreground">最高温度</span>
              <b class="tabular" :style="{ color: statusColor[tempStatus(s.disk.temp, st.diskTempThreshold)] }">{{ fmtTemp(s.disk.temp) }}</b>
            </div>
          </div>
          <div class="mt-3 space-y-1 text-xs">
            <div v-for="t in s.temps.filter((t) => t.hwType === 'Storage')" :key="t.hardware + t.name" class="flex justify-between">
              <span class="truncate text-muted-foreground">{{ t.hardware }} · {{ t.name }}</span><span class="tabular">{{ fmtTemp(t.value) }}</span>
            </div>
          </div>
        </Card>
      </div>

      <div class="grid grid-cols-2 gap-4">
        <!-- 温度 -->
        <Card id="temps" title="温度监控" :subtitle="`${s.temps.length} 个温度传感器`">
          <div v-if="!s.temps.length" class="rounded-md bg-muted p-4 text-xs text-muted-foreground">
            温度：暂不支持读取。当前硬件或权限下没有可用的温度传感器，FanGuard 不会显示虚构数据；以管理员身份运行可读取更多传感器。
          </div>
          <div v-else class="max-h-[320px] space-y-1 overflow-auto text-xs">
            <div v-for="t in s.temps" :key="t.hardware + t.name" class="grid grid-cols-[1fr_120px_60px] items-center gap-3 py-0.5">
              <span class="truncate"><span class="text-muted-foreground">{{ t.hardware }} ·</span> {{ t.name }}</span>
              <Progress :value="t.value" :max="110" :color="statusColor[tempStatus(t.value, t.hwType.startsWith('Gpu') ? st.gpuTempThreshold : t.hwType === 'Storage' ? st.diskTempThreshold : st.cpuTempThreshold)]" height="4px" />
              <span class="text-right tabular">{{ t.value.toFixed(0) }}°C</span>
            </div>
          </div>
        </Card>

        <!-- 风扇 -->
        <Card id="fans" title="风扇监控" subtitle="V1.0 仅监控，不修改风扇转速">
          <div v-if="!s.fans.length" class="rounded-md bg-muted p-4 text-xs text-muted-foreground">
            风扇转速：暂不支持读取。Windows 没有统一的风扇转速接口，当前硬件未通过 LibreHardwareMonitor 暴露 RPM 数据。FanGuard 不会显示虚构的转速，仍可通过温度和负载进行诊断。
          </div>
          <div v-else class="space-y-3 text-xs">
            <div v-for="(list, kind) in fansByKind" :key="kind">
              <template v-if="list.length">
                <div class="mb-1 font-medium text-muted-foreground">{{ fanKind[kind] }}</div>
                <div v-for="f in list" :key="f.hardware + f.name" class="grid grid-cols-[1fr_120px_80px] items-center gap-3 py-0.5">
                  <span class="truncate"><span class="text-muted-foreground">{{ f.hardware }} ·</span> {{ f.name }}</span>
                  <Progress :value="f.rpm" :max="Math.max(st.fanRpmThreshold * 1.3, f.rpm)" :color="statusColor[fanStatus(f.rpm, st.fanRpmThreshold)]" height="4px" />
                  <span class="text-right tabular">{{ Math.round(f.rpm) }} RPM</span>
                </div>
              </template>
            </div>
          </div>
          <div class="mt-3">
            <div class="mb-1 text-xs text-muted-foreground">风扇转速历史变化</div>
            <RealtimeChart :initial="['fan']" :only="['fan', 'cpuTemp']" height="150px" :show-toggles="true" />
          </div>
        </Card>
      </div>

      <!-- 全部传感器 -->
      <Card title="全部传感器" subtitle="LibreHardwareMonitor 原始数据">
        <template #actions>
          <Button size="sm" variant="ghost" @click="showRaw = !showRaw">{{ showRaw ? "收起" : "展开" }}</Button>
        </template>
        <div v-if="showRaw">
          <div v-if="!raw.length" class="py-4 text-xs text-muted-foreground">没有可用的传感器数据</div>
          <div v-for="hw in raw" :key="hw.id" class="border-b border-border/50 py-1 text-xs">
            <button class="flex w-full items-center gap-1.5 py-1 text-left font-medium hover:text-primary" @click="toggle(hw.id)">
              <component :is="expanded.has(hw.id) ? ChevronDown : ChevronRight" class="h-3.5 w-3.5" />
              {{ hw.parentName ? hw.parentName + " / " : "" }}{{ hw.name }}
              <span class="font-normal text-muted-foreground">（{{ hw.type }} · {{ hw.sensors.length }} 个传感器）</span>
            </button>
            <div v-if="expanded.has(hw.id)" class="ml-5 grid grid-cols-[1fr_80px_90px_90px_90px] gap-x-3 py-1">
              <span class="text-muted-foreground">传感器</span><span class="text-muted-foreground">类型</span>
              <span class="text-right text-muted-foreground">当前</span><span class="text-right text-muted-foreground">最小</span><span class="text-right text-muted-foreground">最大</span>
              <template v-for="sn in hw.sensors" :key="sn.id">
                <span class="truncate">{{ sn.name }}</span>
                <span class="text-muted-foreground">{{ typeName[sn.type] ?? sn.type }}</span>
                <span class="text-right tabular">{{ sn.value === null ? "-" : sn.value.toFixed(1) + (sn.unit ? " " + sn.unit : unit[sn.type] ?? "") }}</span>
                <span class="text-right tabular text-muted-foreground">{{ sn.min === null ? "-" : sn.min.toFixed(1) }}</span>
                <span class="text-right tabular text-muted-foreground">{{ sn.max === null ? "-" : sn.max.toFixed(1) }}</span>
              </template>
            </div>
          </div>
        </div>
      </Card>
    </div>
  </div>
</template>
