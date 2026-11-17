<script setup lang="ts">
import { computed, ref } from "vue";
import EChart from "./EChart.vue";
import Segmented from "./ui/Segmented.vue";
import { store } from "../store";
import { SERIES, lineSeries, markerColor, timeAxis, tooltipBase, tooltipFormatter, valueAxis } from "../chart";
import type { SysPoint } from "../types";

const props = withDefaults(
  defineProps<{ initial?: string[]; height?: string; showToggles?: boolean; showRange?: boolean; only?: string[] }>(),
  { initial: () => ["cpu", "gpu", "cpuTemp", "fan"], height: "260px", showToggles: true, showRange: true },
);

const selected = ref<string[]>([...props.initial]);
const range = ref("1");
const ranges = [
  { label: "1 分钟", value: "1" },
  { label: "5 分钟", value: "5" },
  { label: "15 分钟", value: "15" },
  { label: "30 分钟", value: "30" },
];

const available = computed(() => SERIES.filter((s) => !props.only || props.only.includes(s.key)));

function toggle(key: string) {
  const i = selected.value.indexOf(key);
  if (i >= 0) {
    if (selected.value.length > 1) selected.value.splice(i, 1);
  } else selected.value.push(key);
}

const option = computed(() => {
  // 依赖版本号以在新数据到达时重新计算
  void store.version;
  const now = store.snapshot?.ts ?? Date.now();
  const from = now - Number(range.value) * 60_000;
  const pts = store.points.filter((p) => p.ts >= from);
  const defs = available.value.filter((s) => selected.value.includes(s.key));
  const axes: string[] = [];
  for (const d of defs) if (!axes.includes(d.axis)) axes.push(d.axis);
  const yAxis = axes.map((a, i) => {
    const pos = i === 0 ? "left" : "right";
    const offset = i >= 2 ? 52 : 0;
    if (a === "pct") return valueAxis({ unit: "%", max: 100, position: pos, offset });
    if (a === "temp") return valueAxis({ unit: "°", position: pos, offset, min: 20 });
    if (a === "watt") return valueAxis({ unit: "W", position: pos, offset });
    return valueAxis({ unit: "", position: pos, offset });
  });
  const units: Record<string, string> = {};
  const series = defs.map((d, idx) => {
    units[d.name] = d.unit;
    const data = pts.map((p) => [p.ts, (p as any)[d.key] as number | null] as [number, number | null]);
    const s: any = lineSeries(d.name, d.color, data, axes.indexOf(d.axis), idx === 0);
    if (idx === 0) {
      const marks = store.markers.filter((m) => m.ts >= from);
      s.markLine = {
        symbol: ["none", "none"],
        silent: false,
        animation: false,
        label: { show: true, formatter: (p: any) => p.data.name, color: "inherit", fontSize: 10, position: "insideEndTop" },
        data: marks.map((m) => ({ xAxis: m.ts, name: m.title, lineStyle: { color: markerColor(m.level), type: "dashed", width: 1 } })),
      };
    }
    return s;
  });
  const rightAxes = axes.length - 1;
  return {
    animation: false,
    grid: { left: 46, right: rightAxes <= 0 ? 16 : rightAxes === 1 ? 52 : 104, top: 18, bottom: 26 },
    tooltip: { ...tooltipBase(), formatter: tooltipFormatter(units) },
    xAxis: { ...timeAxis(), min: from, max: now },
    yAxis,
    series,
  };
});

const missing = computed(() => {
  const last: SysPoint | undefined = store.points[store.points.length - 1];
  void store.version;
  if (!last) return [];
  return available.value.filter((s) => selected.value.includes(s.key) && (last as any)[s.key] === null).map((s) => s.name);
});
</script>

<template>
  <div>
    <div v-if="showToggles || showRange" class="mb-2 flex flex-wrap items-center justify-between gap-2">
      <div v-if="showToggles" class="flex flex-wrap gap-1.5">
        <button
          v-for="s in available"
          :key="s.key"
          class="flex h-6 items-center gap-1.5 rounded-full border px-2.5 text-xs transition-colors"
          :class="selected.includes(s.key) ? 'border-transparent bg-accent text-foreground' : 'border-border text-muted-foreground hover:text-foreground'"
          @click="toggle(s.key)"
        >
          <span class="h-2 w-2 rounded-full" :style="{ background: selected.includes(s.key) ? s.color : 'var(--muted-foreground)' }" />
          {{ s.name }}
        </button>
      </div>
      <Segmented v-if="showRange" v-model="range" :options="ranges" />
    </div>
    <EChart :option="option" :height="height" />
    <div v-if="missing.length" class="mt-1 text-[11px] text-muted-foreground">{{ missing.join("、") }}：暂不支持读取（硬件未提供该传感器数据）</div>
  </div>
</template>
