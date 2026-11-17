<script setup lang="ts">
withDefaults(defineProps<{ value: number | null; max?: number; color?: string; height?: string }>(), {
  max: 100,
  color: "var(--primary)",
  height: "6px",
});
</script>

<template>
  <div class="w-full overflow-hidden rounded-full bg-muted" :style="{ height }">
    <div
      class="h-full rounded-full transition-[width] duration-500"
      :style="{ width: `${value === null ? 0 : Math.min(100, Math.max(0, (value / max) * 100))}%`, background: color }"
    />
  </div>
</template>
