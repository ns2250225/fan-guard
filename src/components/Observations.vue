<script setup lang="ts">
import { TrendingDown, TrendingUp, Loader2, X } from "lucide-vue-next";
import { store, dismissObservation, type Observation } from "../store";

function delta(before: number | null, after: number | null | undefined, unit: string, digits = 0) {
  if (before === null || after === null || after === undefined) return null;
  const d = after - before;
  return { text: `${before.toFixed(digits)}${unit} → ${after.toFixed(digits)}${unit}`, down: d < 0, d };
}

function rows(o: Observation) {
  const a = o.after;
  return [
    { label: "CPU", v: delta(o.before.cpu, a?.cpu, "%") },
    { label: "CPU 温度", v: delta(o.before.cpuTemp, a?.cpuTemp, "°C") },
    { label: "GPU 温度", v: delta(o.before.gpuTemp, a?.gpuTemp, "°C") },
    { label: "风扇", v: delta(o.before.fan, a?.fan, " RPM") },
  ].filter((r) => r.v !== null || !a);
}

function verdict(o: Observation) {
  if (!o.after) return "";
  const cpuDrop = o.before.cpu - o.after.cpu;
  const tDrop = o.before.cpuTemp !== null && o.after.cpuTemp !== null ? o.before.cpuTemp - o.after.cpuTemp : null;
  if (cpuDrop >= 10 || (tDrop !== null && tDrop >= 5)) return "负载/温度已明显下降，问题可能已缓解。";
  if (cpuDrop >= 3) return "负载略有下降，建议继续观察。";
  return "负载与温度未明显下降，可能还有其他原因，建议重新诊断。";
}
</script>

<template>
  <div v-if="store.observations.length" class="space-y-2">
    <div v-for="o in store.observations" :key="o.at" class="rounded-lg border border-border bg-card p-3">
      <div class="flex items-center justify-between">
        <div class="text-xs font-medium">
          结束 <b>{{ o.name }}</b>（PID {{ o.pid }}）后的变化
          <span v-if="!o.after" class="ml-1 inline-flex items-center gap-1 font-normal text-muted-foreground">
            <Loader2 class="h-3 w-3 animate-spin" /> 正在观察 15 秒…
          </span>
        </div>
        <button class="text-muted-foreground hover:text-foreground" @click="dismissObservation(o)"><X class="h-3.5 w-3.5" /></button>
      </div>
      <div v-if="o.after" class="mt-2 flex flex-wrap gap-x-6 gap-y-1 text-xs tabular">
        <span v-for="r in rows(o)" :key="r.label" class="flex items-center gap-1">
          <span class="text-muted-foreground">{{ r.label }}</span>
          <template v-if="r.v">
            <component :is="r.v.down ? TrendingDown : TrendingUp" class="h-3.5 w-3.5" :class="r.v.down ? 'text-ok' : 'text-warn'" />
            {{ r.v.text }}
          </template>
        </span>
      </div>
      <div v-if="o.after" class="mt-1.5 text-xs text-muted-foreground">{{ verdict(o) }}</div>
    </div>
  </div>
</template>
