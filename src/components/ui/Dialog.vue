<script setup lang="ts">
import { onBeforeUnmount, onMounted } from "vue";
import { X } from "lucide-vue-next";

const props = withDefaults(defineProps<{ open: boolean; title?: string; width?: string; closable?: boolean; z?: number }>(), {
  width: "460px",
  closable: true,
});
const emit = defineEmits<{ (e: "close"): void }>();

function onKey(e: KeyboardEvent) {
  if (e.key === "Escape" && props.open && props.closable) emit("close");
}
onMounted(() => window.addEventListener("keydown", onKey));
onBeforeUnmount(() => window.removeEventListener("keydown", onKey));
</script>

<template>
  <Teleport to="body">
    <Transition name="fade">
      <div v-if="open" class="fixed inset-0 flex items-center justify-center bg-black/50 p-6" :style="{ zIndex: z ?? 50 }" @mousedown.self="closable && emit('close')">
        <div
          class="flex max-h-full flex-col overflow-hidden rounded-lg border border-border bg-popover text-foreground shadow-2xl"
          :style="{ width, maxWidth: '100%' }"
        >
          <div v-if="title || $slots.title" class="flex items-center justify-between gap-3 border-b border-border px-4 py-3">
            <div class="min-w-0 text-sm font-semibold"><slot name="title">{{ title }}</slot></div>
            <button v-if="closable" class="rounded p-1 text-muted-foreground hover:bg-accent hover:text-foreground" @click="emit('close')">
              <X class="h-4 w-4" />
            </button>
          </div>
          <div class="min-h-0 flex-1 overflow-auto"><slot /></div>
          <div v-if="$slots.footer" class="flex items-center justify-end gap-2 border-t border-border px-4 py-3">
            <slot name="footer" />
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>
