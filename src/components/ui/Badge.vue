<script setup lang="ts">
withDefaults(defineProps<{ color?: string; solid?: boolean }>(), { color: "var(--muted-foreground)" });
</script>

<template>
  <span
    class="inline-flex items-center gap-1 rounded px-1.5 py-px text-[11px] leading-[18px] font-medium whitespace-nowrap"
    :style="
      solid
        ? { background: color, color: '#fff' }
        : { color, background: `color-mix(in srgb, ${color} 14%, transparent)` }
    "
  >
    <slot />
  </span>
</template>
