<script setup lang="ts">
defineProps<{ modelValue: boolean; disabled?: boolean }>();
const emit = defineEmits<{ (e: "update:modelValue", v: boolean): void }>();
</script>

<template>
  <button
    type="button"
    role="switch"
    :aria-checked="modelValue"
    :disabled="disabled"
    class="relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors disabled:opacity-50"
    :class="modelValue ? 'bg-primary' : 'bg-input'"
    @click="emit('update:modelValue', !modelValue)"
  >
    <span
      class="inline-block h-4 w-4 rounded-full bg-white shadow transition-transform"
      :class="modelValue ? 'translate-x-[18px]' : 'translate-x-0.5'"
    />
  </button>
</template>
