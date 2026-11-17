<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import { save } from "@tauri-apps/plugin-dialog";
import { Download, RefreshCw, Trash2 } from "lucide-vue-next";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Badge from "../components/ui/Badge.vue";
import Segmented from "../components/ui/Segmented.vue";
import Dialog from "../components/ui/Dialog.vue";
import EChart from "../components/EChart.vue";
import { api } from "../api";
import { toast } from "../store";
import { fmtBytes, fmtTime, fmtDuration, levelColor, levelText } from "../format";
import { lineSeries, markerColor, timeAxis, tooltipBase, tooltipFormatter, valueAxis } from "../chart";
import type { HistoryData } from "../types";

const range = ref("1h");
const data = ref<HistoryData | null>(null);
const loading = ref(false);
const confirmClear = ref(false);
const eventFilter = ref("all");

async function load() {
  loading.value = true;
  try {
    data.value = await api.history(range.value);
  } catch (e) {
    toast({ kind: "error", title: "读取历史数据失败", message: String(e) });
  } finally {
    loading.value = false;
  }
}
onMounted(load);
watch(range, load);

const rangeText: Record<string, string> = { "1h": "最近1小时", "24h": "最近24小时", "7d": "最近7天" };

function markLines() {
  const ev = (data.value?.events ?? []).filter((e) => e.eventType !== "异常进程");
  return {
    symbol: ["none", "none"],
    animation: false,
    label: { show: false },
    data: ev.map((e) => ({ xAxis: e.timestamp, name: e.eventType, lineStyle: { color: markerColor(e.level), type: "dashed", width: 1 } })),
    tooltip: { show: true },
  };
}

function chart(defs: { key: keyof HistoryData["hardware"][number]; name: string; color: string; unit: string }[], yUnit: string, max?: number, min?: number) {
  const d = data.value;
  if (!d) return null;
  const units: Record<string, string> = {};
  const series = defs.map((def, i) => {
    units[def.name] = def.unit;
    const s: any = lineSeries(def.name, def.color, d.hardware.map((r) => [r.timestamp, r[def.key] as number | null]), 0, i === 0);
    if (i === 0) s.markLine = markLines();
    return s;
  });
  return {
    animation: false,
    legend: { top: 0, right: 0, textStyle: { color: "inherit", fontSize: 11 }, itemWidth: 12, itemHeight: 8 },
    grid: { left: 46, right: 16, top: 26, bottom: 46 },
    tooltip: { ...tooltipBase(), formatter: tooltipFormatter(units) },
    xAxis: { ...timeAxis(), min: d.from, max: d.to },
    yAxis: [valueAxis({ unit: yUnit, max, min })],
    dataZoom: [{ type: "inside" }, { type: "slider", height: 14, bottom: 6, borderColor: "transparent", showDetail: false }],
    series,
  };
}

const usageChart = computed(() =>
  chart(
    [
      { key: "cpuUsage", name: "CPU 使用率", color: "#3b82f6", unit: "%" },
      { key: "gpuUsage", name: "GPU 使用率", color: "#a855f7", unit: "%" },
      { key: "memoryUsage", name: "内存使用率", color: "#14b8a6", unit: "%" },
    ],
    "%",
    100,
  ),
);
const tempChart = computed(() =>
  chart(
    [
      { key: "cpuTemp", name: "CPU 温度", color: "#f97316", unit: "°C" },
      { key: "gpuTemp", name: "GPU 温度", color: "#ef4444", unit: "°C" },
      { key: "diskTemp", name: "磁盘温度", color: "#84cc16", unit: "°C" },
    ],
    "°",
    undefined,
    20,
  ),
);
const fanChart = computed(() => chart([{ key: "fanRpm", name: "风扇转速", color: "#eab308", unit: "RPM" }], ""));
const powerChart = computed(() =>
  chart(
    [
      { key: "cpuPower", name: "CPU 功耗", color: "#06b6d4", unit: "W" },
      { key: "gpuPower", name: "GPU 功耗", color: "#ec4899", unit: "W" },
    ],
    "W",
  ),
);

const hasTemp = computed(() => data.value?.hardware.some((r) => r.cpuTemp !== null || r.gpuTemp !== null || r.diskTemp !== null));
const hasFan = computed(() => data.value?.hardware.some((r) => r.fanRpm !== null));
const hasPower = computed(() => data.value?.hardware.some((r) => r.cpuPower !== null || r.gpuPower !== null));

const events = computed(() => {
  const ev = data.value?.events ?? [];
  if (eventFilter.value === "system") return ev.filter((e) => e.eventType !== "异常进程");
  if (eventFilter.value === "process") return ev.filter((e) => e.eventType === "异常进程");
  return ev;
});

const statusText: Record<string, string> = { active: "进行中", resolved: "已结束", ignored: "已忽略" };

async function exportCsv(kind: string, label: string) {
  const stamp = new Date().toISOString().slice(0, 10);
  const path = await save({
    title: `导出${label}`,
    defaultPath: `FanGuard-${label}-${rangeText[range.value]}-${stamp}.csv`,
    filters: [{ name: "CSV 文件", extensions: ["csv"] }],
  });
  if (!path) return;
  try {
    const n = await api.exportCsv(kind, range.value, path);
    toast({ kind: "success", title: "导出成功", message: `已导出 ${n} 条记录到 ${path}` });
  } catch (e) {
    toast({ kind: "error", title: "导出失败", message: String(e) });
  }
}

async function clearAll() {
  confirmClear.value = false;
  try {
    await api.clearHistory();
    toast({ kind: "success", title: "历史记录已清空" });
    load();
  } catch (e) {
    toast({ kind: "error", title: "清空失败", message: String(e) });
  }
}
</script>

<template>
  <div class="h-full overflow-auto">
    <div class="mx-auto max-w-[1400px] space-y-4 p-5">
      <div class="flex items-center justify-between">
        <div>
          <h1 class="text-lg font-semibold">历史记录</h1>
          <div class="text-xs text-muted-foreground">本地 SQLite 保存，可还原风扇异常期间的硬件状态；虚线为异常事件发生时间</div>
        </div>
        <div class="flex items-center gap-2">
          <Segmented
            v-model="range"
            :options="[
              { label: '最近 1 小时', value: '1h' },
              { label: '最近 24 小时', value: '24h' },
              { label: '最近 7 天', value: '7d' },
            ]"
          />
          <Button size="sm" variant="outline" :loading="loading" @click="load"><RefreshCw class="h-3.5 w-3.5" /> 刷新</Button>
        </div>
      </div>

      <Card title="CSV 导出" subtitle="导出所选时间范围内的数据（UTF-8，可用 Excel 打开）">
        <div class="flex flex-wrap gap-2">
          <Button size="sm" variant="secondary" @click="exportCsv('hardware', '硬件历史')"><Download class="h-3.5 w-3.5" /> 硬件历史</Button>
          <Button size="sm" variant="secondary" @click="exportCsv('process', '进程历史')"><Download class="h-3.5 w-3.5" /> 进程历史</Button>
          <Button size="sm" variant="secondary" @click="exportCsv('events', '异常事件')"><Download class="h-3.5 w-3.5" /> 异常事件</Button>
          <Button size="sm" variant="secondary" @click="exportCsv('actions', '进程结束记录')"><Download class="h-3.5 w-3.5" /> 进程结束记录</Button>
          <div class="flex-1" />
          <Button size="sm" variant="ghost" @click="confirmClear = true"><Trash2 class="h-3.5 w-3.5" /> 清空历史</Button>
        </div>
      </Card>

      <div v-if="data && !data.hardware.length" class="rounded-lg border border-border bg-card p-8 text-center text-muted-foreground">
        所选时间范围内暂无历史数据，FanGuard 运行后会按设置的采样间隔自动记录。
      </div>

      <template v-if="data && data.hardware.length">
        <div class="grid grid-cols-2 gap-4">
          <Card title="CPU / GPU / 内存使用率"><EChart :option="usageChart" height="230px" /></Card>
          <Card title="温度历史曲线">
            <EChart v-if="hasTemp" :option="tempChart" height="230px" />
            <div v-else class="flex h-[230px] items-center justify-center text-xs text-muted-foreground">温度：暂不支持读取（无历史数据）</div>
          </Card>
          <Card title="风扇转速曲线">
            <EChart v-if="hasFan" :option="fanChart" height="230px" />
            <div v-else class="flex h-[230px] items-center justify-center text-xs text-muted-foreground">风扇转速：暂不支持读取（无历史数据）</div>
          </Card>
          <Card title="功耗曲线">
            <EChart v-if="hasPower" :option="powerChart" height="230px" />
            <div v-else class="flex h-[230px] items-center justify-center text-xs text-muted-foreground">功耗：暂不支持读取（无历史数据）</div>
          </Card>
        </div>
      </template>

      <div class="grid grid-cols-[1.3fr_1fr] gap-4">
        <!-- 异常事件时间线 -->
        <Card title="异常事件时间线" :subtitle="`${events.length} 条事件`">
          <template #actions>
            <Segmented
              v-model="eventFilter"
              :options="[
                { label: '全部', value: 'all' },
                { label: '系统异常', value: 'system' },
                { label: '异常进程', value: 'process' },
              ]"
            />
          </template>
          <div v-if="!events.length" class="py-8 text-center text-xs text-muted-foreground">暂无异常事件</div>
          <div class="max-h-[420px] overflow-auto pr-1">
            <div v-for="e in events" :key="e.id" class="relative flex gap-3 pb-3 pl-1">
              <div class="flex flex-col items-center">
                <span class="mt-1 h-2.5 w-2.5 rounded-full" :style="{ background: levelColor[e.level] ?? 'var(--notice)' }" />
                <span class="mt-1 w-px flex-1 bg-border" />
              </div>
              <div class="min-w-0 flex-1 text-xs">
                <div class="flex items-center gap-2">
                  <span class="font-medium">{{ e.eventType }}</span>
                  <Badge :color="levelColor[e.level] ?? 'var(--notice)'">{{ levelText[e.level] ?? e.level }}</Badge>
                  <Badge :color="e.status === 'active' ? 'var(--danger)' : 'var(--muted-foreground)'">{{ statusText[e.status] ?? e.status }}</Badge>
                  <span class="ml-auto text-muted-foreground tabular">{{ fmtTime(e.timestamp) }}</span>
                </div>
                <div class="mt-0.5 text-muted-foreground">{{ e.description }}</div>
                <div class="mt-0.5 text-[11px] text-muted-foreground/80">
                  <template v-if="e.relatedName">关联进程：{{ e.relatedName }} (PID {{ e.relatedPid }}) · </template>
                  <template v-if="e.endedAt">持续 {{ fmtDuration((e.endedAt - e.timestamp) / 1000) }}</template>
                  <template v-else-if="e.status === 'active'">仍在持续</template>
                </div>
              </div>
            </div>
          </div>
        </Card>

        <div class="space-y-4">
          <Card title="高负载进程统计" subtitle="按时间范围内的累计 CPU 占用排序">
            <div v-if="!data?.processes.length" class="py-6 text-center text-xs text-muted-foreground">暂无数据</div>
            <table v-else class="w-full text-xs tabular">
              <thead class="text-muted-foreground">
                <tr class="text-left">
                  <th class="py-1 font-medium">进程</th>
                  <th class="py-1 text-right font-medium">平均 CPU</th>
                  <th class="py-1 text-right font-medium">峰值 CPU</th>
                  <th class="py-1 text-right font-medium">平均内存</th>
                  <th class="py-1 text-right font-medium">平均 GPU</th>
                </tr>
              </thead>
              <tbody>
                <tr v-for="p in data.processes" :key="p.processName" class="border-t border-border/50">
                  <td class="max-w-[160px] truncate py-1">{{ p.processName }}</td>
                  <td class="py-1 text-right">{{ p.avgCpu.toFixed(1) }}%</td>
                  <td class="py-1 text-right">{{ p.maxCpu.toFixed(1) }}%</td>
                  <td class="py-1 text-right">{{ fmtBytes(p.avgMemory) }}</td>
                  <td class="py-1 text-right">{{ p.avgGpu.toFixed(1) }}%</td>
                </tr>
              </tbody>
            </table>
          </Card>

          <Card title="进程结束记录">
            <div v-if="!data?.actions.length" class="py-6 text-center text-xs text-muted-foreground">暂无操作记录</div>
            <div class="max-h-[260px] space-y-1.5 overflow-auto">
              <div v-for="a in data?.actions" :key="a.id" class="rounded-md bg-muted px-2.5 py-1.5 text-xs">
                <div class="flex justify-between">
                  <span><b>{{ a.processName }}</b> <span class="text-muted-foreground">PID {{ a.pid }} · {{ a.action }}</span></span>
                  <span class="text-muted-foreground tabular">{{ fmtTime(a.timestamp) }}</span>
                </div>
                <div class="mt-0.5" :class="a.result.startsWith('成功') ? 'text-ok' : 'text-danger'">{{ a.result }}</div>
              </div>
            </div>
          </Card>
        </div>
      </div>
    </div>

    <Dialog :open="confirmClear" title="清空历史记录" @close="confirmClear = false">
      <div class="p-4 text-[13px]">确定清空全部硬件历史、进程历史、异常事件与操作记录？此操作无法撤销。</div>
      <template #footer>
        <Button variant="secondary" @click="confirmClear = false">取消</Button>
        <Button variant="destructive" @click="clearAll">清空</Button>
      </template>
    </Dialog>
  </div>
</template>
