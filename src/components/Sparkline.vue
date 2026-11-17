<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(
  defineProps<{ values: (number | null)[]; max?: number; color?: string; height?: number; fill?: boolean }>(),
  { color: "var(--primary)", height: 36, fill: true },
);

const W = 200;
const path = computed(() => {
  const vals = props.values;
  if (vals.length < 2) return { line: "", area: "" };
  const nums = vals.filter((v): v is number => v !== null);
  const max = props.max ?? Math.max(1, ...nums) * 1.1;
  const h = props.height;
  const step = W / (vals.length - 1);
  let line = "";
  let started = false;
  let firstX = 0;
  let lastX = 0;
  vals.forEach((v, i) => {
    if (v === null) return;
    const x = i * step;
    const y = h - Math.min(1, v / max) * (h - 2) - 1;
    line += `${started ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)} `;
    if (!started) firstX = x;
    started = true;
    lastX = x;
  });
  const area = started ? `${line} L${lastX.toFixed(1)},${h} L${firstX.toFixed(1)},${h} Z` : "";
  return { line, area };
});
</script>

<template>
  <svg :viewBox="`0 0 ${W} ${height}`" preserveAspectRatio="none" class="block w-full" :style="{ height: `${height}px` }">
    <path v-if="fill && path.area" :d="path.area" :fill="color" fill-opacity="0.12" />
    <path :d="path.line" fill="none" :stroke="color" stroke-width="1.5" vector-effect="non-scaling-stroke" />
  </svg>
</template>
