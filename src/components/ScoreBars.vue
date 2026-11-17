<script setup lang="ts">
import type { ScoreBreakdown } from "../types";
import Progress from "./ui/Progress.vue";

const props = defineProps<{ b: ScoreBreakdown }>();

const rows = () => {
  const b = props.b;
  return [
    { label: "CPU 使用率", value: b.cpu, weight: b.weights[0], note: `窗口平均 ${b.avgCpu}%` },
    {
      label: "GPU 使用率",
      value: b.gpu,
      weight: b.weights[1],
      note: b.gpuAvailable ? `窗口平均 ${b.avgGpu}%` : "数据不可用",
    },
    { label: "内存占用及变化", value: b.memory, weight: b.weights[2], note: "" },
    {
      label: "CPU/GPU 功耗关联",
      value: b.power,
      weight: b.weights[3],
      note: b.powerSource === "power" ? "基于功耗" : b.powerSource === "temperature" ? "功耗不可用，以温度估算" : "数据不可用",
    },
    { label: "持续高负载时间", value: b.sustain, weight: b.weights[4], note: `${b.sustainedSecs} 秒` },
  ];
};
</script>

<template>
  <div class="space-y-1.5 text-xs">
    <div v-for="r in rows()" :key="r.label" class="grid grid-cols-[110px_1fr_92px] items-center gap-2">
      <span class="text-muted-foreground">{{ r.label }}</span>
      <Progress :value="r.weight > 0 ? r.value : null" :color="r.weight > 0 ? 'var(--primary)' : 'var(--muted-foreground)'" height="5px" />
      <span class="tabular text-right" :class="r.weight === 0 ? 'text-muted-foreground' : ''">
        <template v-if="r.weight > 0">{{ r.value.toFixed(0) }} · 权重 {{ r.weight }}%</template>
        <template v-else>不可用</template>
      </span>
      <span v-if="r.note" class="col-span-3 -mt-1 pl-[118px] text-[11px] text-muted-foreground/80">{{ r.note }}</span>
    </div>
  </div>
</template>
