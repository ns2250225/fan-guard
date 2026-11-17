<script setup lang="ts">
import { onMounted } from "vue";
import { useRouter } from "vue-router";
import TitleBar from "./components/TitleBar.vue";
import Sidebar from "./components/Sidebar.vue";
import Toasts from "./components/Toasts.vue";
import KillDialog from "./components/KillDialog.vue";
import ProcessDetailDialog from "./components/ProcessDetailDialog.vue";
import ProcessMenu from "./components/ProcessMenu.vue";
import { initStore, store, toast } from "./store";

const router = useRouter();

onMounted(async () => {
  try {
    await initStore((path, action) => {
      store.navigateAction = action;
      router.push({ path, query: action === "high" ? { filter: "high" } : action ? { action } : {} });
    });
  } catch (e) {
    toast({ kind: "error", title: "初始化失败", message: String(e) }, 10000);
  }
});

// 禁用浏览器默认右键菜单
window.addEventListener("contextmenu", (e) => {
  const t = e.target as HTMLElement;
  if (!t.closest("input, textarea, .selectable")) e.preventDefault();
});
</script>

<template>
  <div class="flex h-full flex-col">
    <TitleBar />
    <div class="flex min-h-0 flex-1">
      <Sidebar />
      <main class="min-w-0 flex-1 overflow-hidden">
        <div v-if="!store.ready" class="flex h-full items-center justify-center text-muted-foreground">正在启动监控…</div>
        <RouterView v-else />
      </main>
    </div>
    <KillDialog />
    <ProcessDetailDialog />
    <ProcessMenu />
    <Toasts />
  </div>
</template>
