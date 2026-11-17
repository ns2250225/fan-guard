<script setup lang="ts">
import { ChevronDown } from "lucide-vue-next";

defineProps<{
  modelValue: string | number;
  options: { label: string; value: string | number }[];
  size?: "sm" | "md";
}>();
const emit = defineEmits<{ (e: "update:modelValue", v: any): void }>();

function onChange(e: Event, options: { label: string; value: string | number }[]) {
  const idx = (e.target as HTMLSelectElement).selectedIndex;
  emit("update:modelValue", options[idx].value);
}
</script>

<template>
  <div class="relative inline-flex">
    <select
      class="appearance-none rounded-md border border-border bg-card pr-7 pl-2.5 text-foreground outline-none focus:ring-2 focus:ring-primary/40"
      :class="size === 'sm' ? 'h-7 text-xs' : 'h-8 text-[13px]'"
      :value="modelValue"
      @change="onChange($event, options)"
    >
      <option v-for="o in options" :key="o.value" :value="o.value" class="bg-popover">{{ o.label }}</option>
    </select>
    <ChevronDown class="pointer-events-none absolute top-1/2 right-2 h-3.5 w-3.5 -translate-y-1/2 text-muted-foreground" />
  </div>
</template>
