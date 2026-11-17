<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from "vue";
import { FolderOpen, FileText, ShieldCheck, RefreshCw, CircleStop, Zap, GitBranch, ShieldOff, BadgeCheck, BadgeAlert } from "lucide-vue-next";
import Dialog from "./ui/Dialog.vue";
import Button from "./ui/Button.vue";
import Badge from "./ui/Badge.vue";
import Segmented from "./ui/Segmented.vue";
import EChart from "./EChart.vue";
import ScoreBars from "./ScoreBars.vue";
import { api } from "../api";
import { store, openKill, openDetail, toggleWhitelist, isWhitelisted, toast } from "../store";
import { fmtBytes, fmtTime, fmtUptime, fmtRate, gradeColor, gradeText } from "../format";
import { lineSeries, timeAxis, tooltipBase, tooltipFormatter, valueAxis } from "../chart";
import type { ProcessDetail, SignatureInfo, ThreadInfo, FileInfo, KillMode } from "../types";

const detail = ref<ProcessDetail | null>(null);
const file = ref<FileInfo | null>(null);
const sig = ref<SignatureInfo | null>(null);
const sigLoading = ref(false);
const threads = ref<ThreadInfo[] | null>(null);
const threadsLoading = ref(false);
const tab = ref("overview");
const error = ref("");
let timer: number | undefined;

const open = computed(() => !!store.detail);

async function load(withFile: boolean) {
  const d = store.detail;
  if (!d) return;
  try {
    const r = await api.processDetail(d.pid, d.createTime, withFile);
    detail.value = r;
    if (withFile) file.value = r.file;
    error.value = "";
  } catch (e) {
    error.value = String(e);
  }
}

async function loadSignature() {
  const p = detail.value?.info.path;
  if (!p) return;
  sigLoading.value = true;
  try {
    sig.value = await api.signature(p);
  } finally {
    sigLoading.value = false;
  }
}

async function loadThreads() {
  const d = store.detail;
  if (!d) return;
  threadsLoading.value = true;
  try {
    threads.value = await api.threads(d.pid, d.createTime);
  } catch (e) {
    toast({ kind: "error", title: "读取线程失败", message: String(e) });
  } finally {
    threadsLoading.value = false;
  }
}

watch(
  () => store.detail,
  async (d) => {
    clearInterval(timer);
    detail.value = null;
    file.value = null;
    sig.value = null;
    threads.value = null;
    error.value = "";
    tab.value = "overview";
    if (!d) return;
    await load(true);
    timer = window.setInterval(() => load(false), 1000);
  },
);

watch(tab, (t) => {
  if (t === "file" && !sig.value && !sigLoading.value) loadSignature();
  if (t === "threads" && !threads.value) loadThreads();
});

onBeforeUnmount(() => clearInterval(timer));

function close() {
  store.detail = null;
}

const info = computed(() => detail.value?.info);

const chartOption = computed(() => {
  const d = detail.value;
  if (!d) return null;
  const series: any[] = [lineSeries("CPU", "#3b82f6", d.history.map((h) => [h[0], h[1]]), 0, true)];
  if (d.gpuSupported) series.push(lineSeries("GPU", "#a855f7", d.history.map((h) => [h[0], h[2]]), 0, false));
  return {
    animation: false,
    grid: { left: 44, right: 12, top: 16, bottom: 24 },
    tooltip: { ...tooltipBase(), formatter: tooltipFormatter({ CPU: "%", GPU: "%" }) },
    xAxis: timeAxis(),
    yAxis: [valueAxis({ unit: "%", max: Math.max(10, Math.ceil(Math.max(...d.history.map((h) => Math.max(h[1], h[2])), 1) / 10) * 10) })],
    series,
  };
});

const kindColor = computed(() => {
  const k = info.value?.kind;
  return k === "critical" ? "var(--ok)" : k === "service" ? "var(--notice)" : k === "system" ? "var(--primary)" : "var(--muted-foreground)";
});

function kill(mode: KillMode) {
  if (info.value) openKill(info.value, mode);
}

async function openLocation() {
  if (!info.value?.path) return;
  try {
    await api.openFileLocation(info.value.path);
  } catch (e) {
    toast({ kind: "error", title: "打开文件位置失败", message: String(e) });
  }
}

async function properties() {
  if (!info.value?.path) return;
  try {
    await api.showFileProperties(info.value.path);
  } catch (e) {
    toast({ kind: "error", title: "无法打开文件属性", message: String(e) });
  }
}

const sigColor = computed(() => {
  switch (sig.value?.status) {
    case "valid":
      return "var(--ok)";
    case "notSigned":
      return "var(--notice)";
    case "hashMismatch":
    case "notTrusted":
      return "var(--danger)";
    default:
      return "var(--muted-foreground)";
  }
});

const tabs = [
  { label: "概览", value: "overview" },
  { label: "文件与签名", value: "file" },
  { label: "线程", value: "threads" },
  { label: "父子进程", value: "tree" },
  { label: "GPU 引擎", value: "gpu" },
];
</script>

<template>
  <Dialog :open="open" width="760px" @close="close">
    <template #title>
      <div v-if="info" class="flex items-center gap-2">
        <span class="selectable">{{ info.name }}</span>
        <span class="font-normal text-muted-foreground">PID {{ info.pid }}</span>
        <Badge :color="kindColor">{{ detail?.kindText }}</Badge>
        <Badge v-if="!detail?.alive" color="var(--danger)">已退出</Badge>
        <Badge v-if="info.whitelisted" color="var(--primary)">白名单</Badge>
      </div>
      <span v-else>进程详情</span>
    </template>

    <div class="p-4">
      <div v-if="error && !detail" class="py-10 text-center text-muted-foreground">{{ error }}</div>
      <div v-else-if="!detail" class="py-10 text-center text-muted-foreground">加载中…</div>
      <template v-else-if="info">
        <div class="mb-3"><Segmented v-model="tab" :options="tabs" /></div>

        <!-- 概览 -->
        <div v-if="tab === 'overview'" class="space-y-4">
          <div class="grid grid-cols-4 gap-2">
            <div class="rounded-md bg-muted p-2.5">
              <div class="text-xs text-muted-foreground">CPU（整机）</div>
              <div class="mt-0.5 text-lg font-semibold tabular">{{ info.cpu.toFixed(1) }}%</div>
            </div>
            <div class="rounded-md bg-muted p-2.5">
              <div class="text-xs text-muted-foreground">内存</div>
              <div class="mt-0.5 text-lg font-semibold tabular">{{ fmtBytes(info.memory) }}</div>
            </div>
            <div class="rounded-md bg-muted p-2.5">
              <div class="text-xs text-muted-foreground">GPU</div>
              <div class="mt-0.5 text-lg font-semibold tabular">{{ info.gpu === null ? "不可用" : info.gpu.toFixed(1) + "%" }}</div>
            </div>
            <div class="rounded-md bg-muted p-2.5">
              <div class="text-xs text-muted-foreground">显存</div>
              <div class="mt-0.5 text-lg font-semibold tabular">{{ info.gpuMem === null ? "不可用" : fmtBytes(info.gpuMem) }}</div>
            </div>
          </div>

          <div>
            <div class="mb-1 text-xs font-medium text-muted-foreground">CPU 使用率历史（最近 5 分钟）</div>
            <EChart v-if="chartOption" :option="chartOption" height="150px" />
          </div>

          <div class="grid grid-cols-2 gap-x-6 gap-y-1.5 text-xs">
            <div class="flex justify-between"><span class="text-muted-foreground">启动时间</span><span>{{ fmtTime(info.startTime) }}</span></div>
            <div class="flex justify-between"><span class="text-muted-foreground">运行时间</span><span>{{ fmtUptime(info.startTime) }}</span></div>
            <div class="flex justify-between"><span class="text-muted-foreground">父进程 PID</span><span>{{ info.ppid || "-" }}</span></div>
            <div class="flex justify-between"><span class="text-muted-foreground">会话 ID</span><span>{{ info.sessionId }}</span></div>
            <div class="flex justify-between"><span class="text-muted-foreground">线程数</span><span>{{ info.threads }}</span></div>
            <div class="flex justify-between"><span class="text-muted-foreground">句柄数</span><span>{{ info.handles }}</span></div>
            <div class="flex justify-between"><span class="text-muted-foreground">工作集</span><span>{{ fmtBytes(info.workingSet) }}</span></div>
            <div class="flex justify-between"><span class="text-muted-foreground">提交大小</span><span>{{ fmtBytes(info.privateBytes) }}</span></div>
            <div class="flex justify-between"><span class="text-muted-foreground">I/O 读取</span><span>{{ fmtRate(info.ioRead) }}</span></div>
            <div class="flex justify-between"><span class="text-muted-foreground">I/O 写入</span><span>{{ fmtRate(info.ioWrite) }}</span></div>
          </div>
          <div class="text-xs">
            <span class="text-muted-foreground">文件路径：</span>
            <span class="break-all selectable">{{ info.path ?? "无法读取（受保护进程或需要管理员权限）" }}</span>
          </div>

          <div v-if="detail.suspect" class="rounded-md border border-border p-3">
            <div class="mb-2 flex items-center justify-between">
              <div class="text-xs font-medium">异常关联评分</div>
              <div class="flex items-center gap-2">
                <span class="text-lg font-semibold tabular" :style="{ color: gradeColor(detail.score) }">{{ detail.score.toFixed(0) }}</span>
                <Badge :color="gradeColor(detail.score)">{{ gradeText(detail.score) }}</Badge>
              </div>
            </div>
            <ScoreBars v-if="detail.breakdown" :b="detail.breakdown" />
            <ul class="mt-2 list-disc space-y-0.5 pl-4 text-xs text-muted-foreground">
              <li v-for="r in detail.suspect.reasons" :key="r">{{ r }}</li>
            </ul>
            <div class="mt-2 text-[11px] text-muted-foreground/80">该分数为优先排查分数，不代表真实的风扇因果贡献率。CPU 为整机百分比（已按 {{ detail.logicalProcessors }} 个逻辑处理器归一化）。</div>
          </div>
        </div>

        <!-- 文件与签名 -->
        <div v-else-if="tab === 'file'" class="space-y-4 text-xs">
          <div v-if="!info.path" class="rounded-md bg-muted p-3 text-muted-foreground">
            无法读取该进程的文件路径。该进程可能受系统保护，或需要以管理员身份运行 FanGuard。
          </div>
          <template v-else>
            <div class="break-all rounded-md bg-muted p-2.5 font-mono selectable">{{ info.path }}</div>
            <div class="flex gap-2">
              <Button size="sm" variant="outline" @click="openLocation"><FolderOpen class="h-3.5 w-3.5" /> 打开文件所在目录</Button>
              <Button size="sm" variant="outline" @click="properties"><FileText class="h-3.5 w-3.5" /> 查看文件属性</Button>
            </div>
            <div class="grid grid-cols-[90px_1fr] gap-y-1.5">
              <span class="text-muted-foreground">文件描述</span><span>{{ file?.description ?? "-" }}</span>
              <span class="text-muted-foreground">公司</span><span>{{ file?.company ?? "-" }}</span>
              <span class="text-muted-foreground">产品名称</span><span>{{ file?.product ?? "-" }}</span>
              <span class="text-muted-foreground">文件版本</span><span>{{ file?.version ?? "-" }}</span>
              <span class="text-muted-foreground">原始文件名</span><span>{{ file?.originalName ?? "-" }}</span>
              <span class="text-muted-foreground">版权</span><span>{{ file?.copyright ?? "-" }}</span>
              <span class="text-muted-foreground">文件大小</span><span>{{ fmtBytes(file?.size ?? null) }}</span>
              <span class="text-muted-foreground">修改时间</span><span>{{ fmtTime(file?.modified ?? null) }}</span>
            </div>
            <div class="rounded-md border border-border p-3">
              <div class="mb-2 flex items-center justify-between">
                <div class="flex items-center gap-1.5 font-medium"><ShieldCheck class="h-3.5 w-3.5" /> 数字签名</div>
                <Button size="xs" variant="ghost" :loading="sigLoading" @click="loadSignature"><RefreshCw class="h-3 w-3" /> 重新验证</Button>
              </div>
              <div v-if="sigLoading && !sig" class="text-muted-foreground">正在验证签名…</div>
              <div v-else-if="sig" class="grid grid-cols-[90px_1fr] gap-y-1.5">
                <span class="text-muted-foreground">签名状态</span>
                <span class="flex items-center gap-1 font-medium" :style="{ color: sigColor }">
                  <BadgeCheck v-if="sig.status === 'valid'" class="h-3.5 w-3.5" />
                  <BadgeAlert v-else class="h-3.5 w-3.5" />
                  {{ sig.statusText }}
                </span>
                <span class="text-muted-foreground">签名者</span><span class="selectable">{{ sig.signer ?? "-" }}</span>
                <span class="text-muted-foreground">颁发者</span><span class="selectable">{{ sig.issuer ?? "-" }}</span>
                <span class="text-muted-foreground">签名类型</span><span>{{ sig.signatureType ?? "-" }}</span>
                <span class="text-muted-foreground">证书有效期至</span><span>{{ sig.notAfter ?? "-" }}</span>
                <span class="text-muted-foreground">指纹</span><span class="font-mono break-all selectable">{{ sig.thumbprint ?? "-" }}</span>
              </div>
            </div>
          </template>
        </div>

        <!-- 线程 -->
        <div v-else-if="tab === 'threads'" class="text-xs">
          <div class="mb-2 flex items-center justify-between">
            <span class="text-muted-foreground">共 {{ threads?.length ?? 0 }} 个线程（按累计 CPU 时间排序）</span>
            <Button size="xs" variant="ghost" :loading="threadsLoading" @click="loadThreads"><RefreshCw class="h-3 w-3" /> 刷新</Button>
          </div>
          <div class="max-h-[380px] overflow-auto rounded-md border border-border">
            <table class="w-full tabular">
              <thead class="sticky top-0 bg-popover text-muted-foreground">
                <tr class="border-b border-border text-left">
                  <th class="px-2 py-1.5 font-medium">TID</th>
                  <th class="px-2 py-1.5 text-right font-medium">CPU 时间</th>
                  <th class="px-2 py-1.5 text-right font-medium">优先级</th>
                  <th class="px-2 py-1.5 text-right font-medium">上下文切换</th>
                  <th class="px-2 py-1.5 font-medium">状态</th>
                  <th class="px-2 py-1.5 font-medium">启动时间</th>
                  <th class="px-2 py-1.5 font-medium">起始地址</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="t in threads" :key="t.tid" class="border-b border-border/50">
                  <td class="px-2 py-1">{{ t.tid }}</td>
                  <td class="px-2 py-1 text-right">{{ (t.cpuTimeMs / 1000).toFixed(2) }} s</td>
                  <td class="px-2 py-1 text-right">{{ t.priority }}</td>
                  <td class="px-2 py-1 text-right">{{ t.contextSwitches.toLocaleString() }}</td>
                  <td class="px-2 py-1">{{ t.state }}<span v-if="t.waitReason" class="text-muted-foreground">（{{ t.waitReason }}）</span></td>
                  <td class="px-2 py-1">{{ fmtTime(t.startTime, false) }}</td>
                  <td class="px-2 py-1 font-mono">{{ t.startAddress }}</td>
                </tr>
              </tbody>
            </table>
          </div>
        </div>

        <!-- 父子进程 -->
        <div v-else-if="tab === 'tree'" class="space-y-3 text-xs">
          <div>
            <div class="mb-1 font-medium text-muted-foreground">父进程</div>
            <button
              v-if="detail.parent"
              class="flex w-full items-center justify-between rounded-md border border-border px-3 py-2 hover:bg-accent"
              @click="openDetail(detail.parent.pid, detail.parent.createTime)"
            >
              <span>{{ detail.parent.name }} <span class="text-muted-foreground">PID {{ detail.parent.pid }}</span></span>
              <span class="tabular">CPU {{ detail.parent.cpu.toFixed(1) }}% · {{ fmtBytes(detail.parent.memory) }}</span>
            </button>
            <div v-else class="text-muted-foreground">父进程 PID {{ info.ppid }} 已退出或不可用</div>
          </div>
          <div>
            <div class="mb-1 font-medium text-muted-foreground">子进程（{{ detail.children.length }}）</div>
            <div class="max-h-[320px] space-y-1 overflow-auto">
              <button
                v-for="c in detail.children"
                :key="c.pid"
                class="flex w-full items-center justify-between rounded-md border border-border px-3 py-2 hover:bg-accent"
                @click="openDetail(c.pid, c.createTime)"
              >
                <span>{{ c.name }} <span class="text-muted-foreground">PID {{ c.pid }}</span></span>
                <span class="tabular">CPU {{ c.cpu.toFixed(1) }}% · {{ fmtBytes(c.memory) }}<template v-if="c.gpu !== null"> · GPU {{ c.gpu.toFixed(0) }}%</template></span>
              </button>
              <div v-if="!detail.children.length" class="text-muted-foreground">没有子进程</div>
            </div>
          </div>
        </div>

        <!-- GPU 引擎 -->
        <div v-else-if="tab === 'gpu'" class="text-xs">
          <div v-if="!detail.gpuSupported" class="rounded-md bg-muted p-3 text-muted-foreground">
            当前系统不支持 GPU 性能计数器，无法获取进程关联的 GPU 引擎。
          </div>
          <template v-else>
            <div class="mb-2 text-muted-foreground">显存占用：{{ fmtBytes(info.gpuMem) }}</div>
            <div v-if="!detail.gpuEngines.length" class="rounded-md bg-muted p-3 text-muted-foreground">该进程当前没有使用 GPU 引擎。</div>
            <div v-for="e in detail.gpuEngines" :key="e.adapter + e.engine" class="flex items-center justify-between border-b border-border/50 py-1.5">
              <span>{{ e.engine }} <span class="text-muted-foreground">· {{ e.adapter }}</span></span>
              <span class="tabular">{{ e.usage.toFixed(1) }}%</span>
            </div>
          </template>
        </div>
      </template>
    </div>

    <template #footer>
      <template v-if="info && detail?.alive">
        <Button size="sm" variant="ghost" @click="toggleWhitelist(info.name)">
          <component :is="isWhitelisted(info.name) ? ShieldOff : ShieldCheck" class="h-3.5 w-3.5" />
          {{ isWhitelisted(info.name) ? "移出白名单" : "添加到白名单" }}
        </Button>
        <div class="flex-1" />
        <template v-if="info.kind !== 'critical'">
          <Button size="sm" variant="secondary" @click="kill('graceful')"><CircleStop class="h-3.5 w-3.5" /> 结束进程</Button>
          <Button size="sm" variant="secondary" @click="kill('tree')"><GitBranch class="h-3.5 w-3.5" /> 结束进程树</Button>
          <Button size="sm" variant="destructive" @click="kill('force')"><Zap class="h-3.5 w-3.5" /> 强制结束</Button>
        </template>
        <Badge v-else color="var(--ok)"><ShieldCheck class="h-3 w-3" /> 关键系统进程，已受保护</Badge>
      </template>
      <Button v-else size="sm" variant="secondary" @click="close">关闭</Button>
    </template>
  </Dialog>
</template>
