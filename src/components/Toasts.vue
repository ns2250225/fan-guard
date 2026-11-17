<script setup lang="ts">
import { CheckCircle2, Info, TriangleAlert, XCircle, X } from "lucide-vue-next";
import { store, dismissToast } from "../store";

const icons = { success: CheckCircle2, error: XCircle, info: Info, warning: TriangleAlert };
const colors = { success: "var(--ok)", error: "var(--danger)", info: "var(--primary)", warning: "var(--warn)" };
</script>

<template>
  <div class="pointer-events-none fixed right-4 bottom-4 z-[80] flex w-[360px] flex-col gap-2">
    <TransitionGroup name="toast">
      <div
        v-for="t in store.toasts"
        :key="t.id"
        class="pointer-events-auto flex gap-3 rounded-lg border border-border bg-popover p-3 shadow-xl"
      >
        <component :is="icons[t.kind]" class="mt-0.5 h-4 w-4 shrink-0" :style="{ color: colors[t.kind] }" />
        <div class="min-w-0 flex-1">
          <div class="text-[13px] font-medium">{{ t.title }}</div>
          <div v-if="t.message" class="mt-0.5 text-xs break-words text-muted-foreground selectable">{{ t.message }}</div>
          <button v-if="t.action" class="mt-1.5 text-xs font-medium text-primary hover:underline" @click="t.action.run(); dismissToast(t.id)">
            {{ t.action.label }}
          </button>
        </div>
        <button class="h-5 w-5 shrink-0 rounded text-muted-foreground hover:text-foreground" @click="dismissToast(t.id)">
          <X class="h-3.5 w-3.5" />
        </button>
      </div>
    </TransitionGroup>
  </div>
</template>
