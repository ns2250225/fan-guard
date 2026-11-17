<script setup lang="ts">
import Sparkline from "./Sparkline.vue";
import Progress from "./ui/Progress.vue";
import { statusColor, statusText, type Status } from "../format";

defineProps<{
  title: string;
  value: string;
  sub?: string;
  status: Status;
  progress: number | null;
  progressMax?: number;
  spark: (number | null)[];
  sparkMax?: number;
  rows?: { label: string; value: string }[];
  unavailable?: boolean;
}>();
const emit = defineEmits<{ (e: "click"): void }>();
</script>

<template>
  <button
    class="group flex min-w-0 flex-col rounded-lg border border-border bg-card p-3.5 text-left transition-colors hover:border-primary/50"
    @click="emit('click')"
  >
    <div class="flex items-center justify-between">
      <span class="text-xs font-medium text-muted-foreground">{{ title }}</span>
      <span class="flex items-center gap-1 text-[11px]" :style="{ color: statusColor[status] }">
        <span class="h-1.5 w-1.5 rounded-full" :style="{ background: statusColor[status] }" />
        {{ statusText[status] }}
      </span>
    </div>
    <div class="mt-1.5 flex items-baseline gap-1.5">
      <span class="text-2xl font-semibold tabular" :class="unavailable ? 'text-base text-muted-foreground' : ''">{{ value }}</span>
      <span v-if="sub" class="truncate text-xs text-muted-foreground">{{ sub }}</span>
    </div>
    <div class="mt-2">
      <Progress :value="progress" :max="progressMax ?? 100" :color="statusColor[status]" height="4px" />
    </div>
    <div class="mt-2 -mx-0.5">
      <Sparkline :values="spark" :max="sparkMax" :color="statusColor[status] === 'var(--muted-foreground)' ? 'var(--primary)' : statusColor[status]" :height="34" />
    </div>
    <div v-if="rows?.length" class="mt-2 space-y-0.5 text-[11px]">
      <div v-for="r in rows" :key="r.label" class="flex justify-between gap-2">
        <span class="text-muted-foreground">{{ r.label }}</span>
        <span class="truncate tabular">{{ r.value }}</span>
      </div>
    </div>
  </button>
</template>
