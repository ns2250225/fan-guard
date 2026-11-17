<script setup lang="ts">
import { computed } from "vue";
import { useRoute } from "vue-router";
import { Activity, Cpu, Gauge, History, ListTree, Settings, Stethoscope, Pause, Play } from "lucide-vue-next";
import { store } from "../store";
import { api } from "../api";
import { levelColor, fmtTemp } from "../format";

const route = useRoute();
const items = [
  { path: "/", label: "仪表盘", icon: Gauge },
  { path: "/processes", label: "进程管理", icon: ListTree },
  { path: "/diagnosis", label: "智能诊断", icon: Stethoscope },
  { path: "/hardware", label: "硬件监控", icon: Cpu },
  { path: "/history", label: "历史记录", icon: History },
  { path: "/settings", label: "系统设置", icon: Settings },
];

const level = computed(() => store.snapshot?.level ?? "normal");
const activeAnomalies = computed(() => store.snapshot?.anomalies.filter((a) => !a.ignored).length ?? 0);

function togglePause() {
  api.setPaused(!store.paused);
}
</script>

<template>
  <aside class="flex w-[200px] shrink-0 flex-col border-r border-border bg-sidebar">
    <nav class="flex flex-col gap-0.5 p-2">
      <RouterLink
        v-for="it in items"
        :key="it.path"
        :to="it.path"
        class="relative flex h-9 items-center gap-3 rounded-md px-3 text-[13px] transition-colors"
        :class="route.path === it.path ? 'bg-accent font-medium text-foreground' : 'text-muted-foreground hover:bg-accent/60 hover:text-foreground'"
      >
        <span v-if="route.path === it.path" class="absolute top-2 bottom-2 left-0 w-[3px] rounded-full bg-primary" />
        <component :is="it.icon" class="h-4 w-4" />
        <span>{{ it.label }}</span>
        <span
          v-if="it.path === '/diagnosis' && activeAnomalies > 0"
          class="ml-auto flex h-[18px] min-w-[18px] items-center justify-center rounded-full px-1 text-[11px] font-semibold text-white"
          :style="{ background: levelColor[level] }"
          >{{ activeAnomalies }}</span
        >
      </RouterLink>
    </nav>

    <div class="mt-auto space-y-2 p-3 text-xs">
      <div class="rounded-md border border-border bg-card p-2.5">
        <div class="mb-1.5 flex items-center gap-1.5 text-muted-foreground">
          <Activity class="h-3.5 w-3.5" />
          <span>当前状态</span>
          <span class="ml-auto font-semibold" :style="{ color: levelColor[level] }">{{ store.snapshot?.levelText ?? "—" }}</span>
        </div>
        <div class="flex justify-between text-muted-foreground">
          <span>CPU</span><span class="tabular text-foreground">{{ store.snapshot ? store.snapshot.cpu.usage.toFixed(0) + "%" : "-" }}</span>
        </div>
        <div class="flex justify-between text-muted-foreground">
          <span>温度</span><span class="tabular text-foreground">{{ store.snapshot?.cpu.temp != null ? fmtTemp(store.snapshot.cpu.temp) : "不可用" }}</span>
        </div>
      </div>
      <button
        class="flex h-8 w-full items-center justify-center gap-2 rounded-md border border-border text-muted-foreground hover:bg-accent hover:text-foreground"
        @click="togglePause"
      >
        <component :is="store.paused ? Play : Pause" class="h-3.5 w-3.5" />
        {{ store.paused ? "恢复监控" : "暂停监控" }}
      </button>
      <div class="text-center text-[11px] text-muted-foreground/70">FanGuard v1.0 · 数据仅保存在本地</div>
    </div>
  </aside>
</template>
