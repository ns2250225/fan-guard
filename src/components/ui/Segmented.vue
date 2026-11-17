<script setup lang="ts">
defineProps<{ modelValue: string; options: { label: string; value: string }[] }>();
const emit = defineEmits<{ (e: "update:modelValue", v: string): void }>();
</script>

<template>
  <div class="inline-flex items-center rounded-md bg-muted p-0.5">
    <button
      v-for="o in options"
      :key="o.value"
      class="h-6 rounded px-2.5 text-xs font-medium transition-colors"
      :class="modelValue === o.value ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground hover:text-foreground'"
      @click="emit('update:modelValue', o.value)"
    >
      {{ o.label }}
    </button>
  </div>
</template>
