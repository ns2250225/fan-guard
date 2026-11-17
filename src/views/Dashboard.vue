<script setup lang="ts">
import { computed } from "vue";
import { useRouter } from "vue-router";
import { Stethoscope } from "lucide-vue-next";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Badge from "../components/ui/Badge.vue";
import HwCard from "../components/HwCard.vue";
import RealtimeChart from "../components/RealtimeChart.vue";
import AlertBanner from "../components/AlertBanner.vue";
import Observations from "../components/Observations.vue";
import { store, openDetail, openKill } from "../store";
import {
  fmtBytes,
  fmtMhz,
  fmtPct,
  fmtRate,
  fmtRpm,
  fmtTemp,
  fmtWatt,
  loadStatus,
  tempStatus,
  fanStatus,
  worst,
  gradeColor,
  levelColor,
  type Status,
} from "../format";

const router = useRouter();
const s = computed(() => store.snapshot);
const st = computed(() => store.settings!);

/** 最近 60 秒数据 */
const recent = computed(() => {
  void store.version;
  const now = s.value?.ts ?? Date.now();
  return store.points.filter((p) => p.ts >= now - 60_000);
});

function sustained(key: "cpu" | "gpu", thr: number) {
  return (s.value?.anomalies ?? []).some((a) => a.code === `${key}_high`) || false;
}

const cpuStatus = computed<Status>(() => {
  const c = s.value?.cpu;
  if (!c) return "unknown";
  return worst(loadStatus(c.usage, st.value.cpuThreshold, sustained("cpu", st.value.cpuThreshold)), tempStatus(c.temp, st.value.cpuTempThreshold));
});
const gpuStatus = computed<Status>(() => {
  if (!s.value || s.value.gpuUsage === null) return "unknown";
  return worst(loadStatus(s.value.gpuUsage, st.value.gpuThreshold, sustained("gpu", st.value.gpuThreshold)), tempStatus(s.value.gpuTemp, st.value.gpuTempThreshold));
});
const memStatus = computed<Status>(() => (s.value ? loadStatus(s.value.memory.usage, 90) : "unknown"));
const diskStatus = computed<Status>(() =>
  worst(
    s.value?.disk.active == null ? "unknown" : loadStatus(s.value.disk.active, 90),
    tempStatus(s.value?.disk.temp, st.value.diskTempThreshold),
  ),
);
const fanSt = computed<Status>(() => fanStatus(s.value?.fanRpm, st.value.fanRpmThreshold));

const primaryGpu = computed(() => s.value?.gpus[0]);

const top = computed(() => {
  const list = s.value?.processes ?? [];
  return [...list]
    .filter((p) => !p.ignored)
    .sort((a, b) => b.cpu - a.cpu)
    .slice(0, 6);
});

const maxDisk = computed(() => {
  const v = store.disk.slice(-60).map((d) => (d.read ?? 0) + (d.write ?? 0));
  void store.version;
  return Math.max(1024 * 1024, ...v);
});

const fanTrend = computed(() => {
  const pts = recent.value.filter((p) => p.fan !== null);
  if (pts.length < 2) return "";
  const d = (pts[pts.length - 1].fan ?? 0) - (pts[0].fan ?? 0);
  if (Math.abs(d) < 50) return "平稳";
  return d > 0 ? `↑ ${Math.round(d)} RPM / 60 秒` : `↓ ${Math.round(-d)} RPM / 60 秒`;
});
</script>

<template>
  <div v-if="s" class="h-full overflow-auto">
    <div class="mx-auto max-w-[1400px] space-y-4 p-5">
      <div class="flex items-center justify-between">
        <div>
          <h1 class="text-lg font-semibold">仪表盘</h1>
          <div class="text-xs text-muted-foreground">
            {{ store.staticInfo?.cpuModel }} · {{ store.staticInfo?.osName }}
          </div>
        </div>
        <div class="flex items-center gap-3">
          <span class="text-xs text-muted-foreground">
            整体状态
            <b :style="{ color: levelColor[s.level] }">{{ s.levelText }}</b>
          </span>
          <Button size="sm" @click="router.push('/diagnosis')"><Stethoscope class="h-3.5 w-3.5" /> 智能诊断</Button>
        </div>
      </div>

      <AlertBanner />
      <Observations />

      <!-- 硬件状态卡片 -->
      <div class="grid grid-cols-5 gap-3">
        <HwCard
          title="CPU"
          :value="fmtPct(s.cpu.usage)"
          :sub="s.cpu.temp !== null ? fmtTemp(s.cpu.temp) : ''"
          :status="cpuStatus"
          :progress="s.cpu.usage"
          :spark="recent.map((p) => p.cpu)"
          :spark-max="100"
          :rows="[
            { label: '温度', value: fmtTemp(s.cpu.temp) },
            { label: '频率', value: fmtMhz(s.cpu.clock) },
            { label: '功耗', value: fmtWatt(s.cpu.power) },
          ]"
          @click="router.push({ path: '/hardware', hash: '#cpu' })"
        />
        <HwCard
          title="GPU"
          :value="s.gpuUsage === null ? '暂不支持读取' : fmtPct(s.gpuUsage)"
          :unavailable="s.gpuUsage === null"
          :sub="s.gpuTemp !== null ? fmtTemp(s.gpuTemp) : ''"
          :status="gpuStatus"
          :progress="s.gpuUsage"
          :spark="recent.map((p) => p.gpu)"
          :spark-max="100"
          :rows="[
            { label: '温度', value: fmtTemp(s.gpuTemp) },
            {
              label: '显存',
              value: primaryGpu?.memUsed != null ? `${fmtBytes(primaryGpu.memUsed)} / ${fmtBytes(primaryGpu.memTotal)}` : '-',
            },
            { label: '功耗', value: fmtWatt(s.gpuPower) },
          ]"
          @click="router.push({ path: '/hardware', hash: '#gpu' })"
        />
        <HwCard
          title="内存"
          :value="fmtPct(s.memory.usage)"
          :sub="`${fmtBytes(s.memory.used)} / ${fmtBytes(s.memory.total)}`"
          :status="memStatus"
          :progress="s.memory.usage"
          :spark="recent.map((p) => p.mem)"
          :spark-max="100"
          :rows="[
            { label: '已使用', value: fmtBytes(s.memory.used) },
            { label: '可用', value: fmtBytes(s.memory.available) },
            { label: '总容量', value: fmtBytes(s.memory.total) },
          ]"
          @click="router.push({ path: '/hardware', hash: '#memory' })"
        />
        <HwCard
          title="磁盘"
          :value="s.disk.active === null ? '不可用' : fmtPct(s.disk.active)"
          :sub="s.disk.temp !== null ? `活动时间 · ${fmtTemp(s.disk.temp)}` : '活动时间'"
          :unavailable="s.disk.active === null"
          :status="diskStatus"
          :progress="s.disk.active"
          :spark="store.disk.slice(-60).map((d) => (d.read ?? 0) + (d.write ?? 0))"
          :spark-max="maxDisk"
          :rows="[
            { label: '读取', value: fmtRate(s.disk.read) },
            { label: '写入', value: fmtRate(s.disk.write) },
            { label: '温度', value: fmtTemp(s.disk.temp) },
          ]"
          @click="router.push({ path: '/hardware', hash: '#disk' })"
        />
        <HwCard
          title="风扇"
          :value="s.fanRpm === null ? '暂不支持读取' : `${Math.round(s.fanRpm)}`"
          :sub="s.fanRpm === null ? '' : 'RPM'"
          :unavailable="s.fanRpm === null"
          :status="fanSt"
          :progress="s.fanRpm"
          :progress-max="Math.max(st.fanRpmThreshold * 1.2, s.fanRpm ?? 0)"
          :spark="recent.map((p) => p.fan)"
          :rows="
            s.fanRpm === null
              ? s.sensor.ec === 'needAdmin' && !store.elevated
                ? [{ label: '说明', value: '笔记本 EC 风扇需管理员权限读取' }]
                : [{ label: '说明', value: '仍可基于温度与负载诊断' }]
              : [
                  { label: '风扇数量', value: `${s.fans.length} 个` },
                  { label: '最高转速', value: fmtRpm(Math.max(...s.fans.map((f) => f.rpm))) },
                  { label: '变化', value: fanTrend || '-' },
                ]
          "
          @click="router.push({ path: '/hardware', hash: '#fans' })"
        />
      </div>

      <!-- 实时负载曲线 -->
      <Card title="实时硬件状态" subtitle="默认展示最近 60 秒，悬停查看具体数值；虚线标记为异常事件发生时间">
        <RealtimeChart />
      </Card>

      <!-- 高负载进程排行 -->
      <Card title="高负载进程排行" :subtitle="`CPU 为整机百分比 · 共 ${s.processCount} 个进程 · ${s.threadCount} 个线程`">
        <template #actions>
          <Button size="sm" variant="ghost" @click="router.push('/processes')">查看全部</Button>
        </template>
        <div class="divide-y divide-border">
          <div
            v-for="p in top"
            :key="p.pid + ':' + p.createTime"
            class="flex h-10 cursor-default items-center gap-3 px-1 hover:bg-accent/50"
            @click="openDetail(p.pid, p.createTime)"
          >
            <span class="w-[200px] truncate font-medium">{{ p.name }}</span>
            <span class="w-16 text-xs text-muted-foreground tabular">PID {{ p.pid }}</span>
            <div class="flex flex-1 items-center gap-2">
              <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-muted">
                <div class="h-full rounded-full bg-primary" :style="{ width: Math.min(100, p.cpu) + '%' }" />
              </div>
              <span class="w-14 text-right tabular">{{ p.cpu.toFixed(1) }}%</span>
            </div>
            <span class="w-20 text-right text-xs tabular text-muted-foreground">{{ fmtBytes(p.memory) }}</span>
            <span class="w-16 text-right text-xs tabular text-muted-foreground">{{ p.gpu === null ? "-" : `GPU ${p.gpu.toFixed(0)}%` }}</span>
            <span class="w-20 text-right">
              <Badge v-if="p.score >= 40" :color="gradeColor(p.score)">评分 {{ p.score.toFixed(0) }}</Badge>
            </span>
            <span class="w-16 text-right" @click.stop>
              <Badge v-if="p.kind === 'critical'" color="var(--ok)">保护</Badge>
              <Button v-else size="xs" variant="secondary" @click="openKill(p, 'graceful')">结束</Button>
            </span>
          </div>
        </div>
      </Card>
    </div>
  </div>
  <div v-else class="flex h-full items-center justify-center text-muted-foreground">正在采集数据…</div>
</template>
