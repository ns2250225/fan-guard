<script setup lang="ts">
defineProps<{ title?: string; subtitle?: string; padded?: boolean }>();
</script>

<template>
  <div class="rounded-lg border border-border bg-card text-card-foreground">
    <div v-if="title || $slots.header || $slots.actions" class="flex items-center justify-between gap-3 px-4 pt-3.5 pb-2">
      <div class="min-w-0">
        <slot name="header">
          <div class="text-[13px] font-semibold">{{ title }}</div>
          <div v-if="subtitle" class="mt-0.5 text-xs text-muted-foreground">{{ subtitle }}</div>
        </slot>
      </div>
      <div class="flex shrink-0 items-center gap-2"><slot name="actions" /></div>
    </div>
    <div :class="padded === false ? '' : 'px-4 pb-4'" class="min-w-0">
      <slot />
    </div>
  </div>
</template>
