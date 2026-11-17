<script setup lang="ts">
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, X, Copy } from "lucide-vue-next";
import { onMounted, ref } from "vue";
import { api } from "../api";
import { store } from "../store";
import Badge from "./ui/Badge.vue";
import logo from "../../src-tauri/icons/64x64.png";

const win = getCurrentWindow();
const maximized = ref(false);

onMounted(async () => {
  maximized.value = await win.isMaximized();
  win.onResized(async () => {
    maximized.value = await win.isMaximized();
  });
});

function minimize() {
  win.minimize();
}
function toggleMax() {
  win.toggleMaximize();
}
function close() {
  api.closeWindow();
}
</script>

<template>
  <div data-tauri-drag-region class="flex h-9 shrink-0 items-center justify-between border-b border-border bg-sidebar pl-3" @dblclick="toggleMax">
    <div data-tauri-drag-region class="flex items-center gap-2 text-[13px] font-semibold">
      <img data-tauri-drag-region :src="logo" class="h-4 w-4" alt="" draggable="false" />
      <span data-tauri-drag-region>FanGuard</span>
      <span data-tauri-drag-region class="font-normal text-muted-foreground">风扇管家</span>
      <Badge v-if="store.elevated" color="var(--notice)">管理员</Badge>
      <Badge v-if="store.paused" color="var(--muted-foreground)">监控已暂停</Badge>
    </div>
    <div class="flex h-full items-center">
      <button class="flex h-full w-11 items-center justify-center text-muted-foreground hover:bg-accent hover:text-foreground" title="最小化" @click="minimize">
        <Minus class="h-4 w-4" />
      </button>
      <button class="flex h-full w-11 items-center justify-center text-muted-foreground hover:bg-accent hover:text-foreground" :title="maximized ? '还原' : '最大化'" @click="toggleMax">
        <Copy v-if="maximized" class="h-3.5 w-3.5" />
        <Square v-else class="h-3.5 w-3.5" />
      </button>
      <button class="flex h-full w-11 items-center justify-center text-muted-foreground hover:bg-[#e81123] hover:text-white" title="关闭" @click="close">
        <X class="h-4 w-4" />
      </button>
    </div>
  </div>
</template>
