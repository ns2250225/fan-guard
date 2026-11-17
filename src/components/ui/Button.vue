<script setup lang="ts">
import { computed } from "vue";

const props = withDefaults(
  defineProps<{
    variant?: "default" | "secondary" | "outline" | "ghost" | "destructive" | "warning";
    size?: "sm" | "md" | "icon" | "xs";
    disabled?: boolean;
    loading?: boolean;
  }>(),
  { variant: "default", size: "md" },
);

const cls = computed(() => {
  const v = {
    default: "bg-primary text-primary-foreground hover:brightness-110",
    secondary: "bg-secondary text-foreground hover:bg-accent",
    outline: "border border-border bg-transparent hover:bg-accent",
    ghost: "bg-transparent hover:bg-accent",
    destructive: "bg-danger text-white hover:brightness-110",
    warning: "bg-warn text-white hover:brightness-110",
  }[props.variant];
  const s = {
    xs: "h-6 px-2 text-xs gap-1",
    sm: "h-7 px-2.5 text-xs gap-1.5",
    md: "h-8 px-3.5 text-[13px] gap-2",
    icon: "h-8 w-8 justify-center",
  }[props.size];
  return [v, s];
});
</script>

<template>
  <button
    :disabled="disabled || loading"
    class="inline-flex items-center justify-center rounded-md font-medium whitespace-nowrap transition-all outline-none select-none focus-visible:ring-2 focus-visible:ring-primary/50 disabled:pointer-events-none disabled:opacity-50 cursor-default"
    :class="cls"
  >
    <svg v-if="loading" class="h-3.5 w-3.5 animate-spin" viewBox="0 0 24 24" fill="none">
      <circle cx="12" cy="12" r="10" stroke="currentColor" stroke-opacity="0.25" stroke-width="4" />
      <path d="M22 12a10 10 0 0 1-10 10" stroke="currentColor" stroke-width="4" />
    </svg>
    <slot />
  </button>
</template>
