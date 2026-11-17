<script setup lang="ts">
import { computed } from "vue";
import { useRouter } from "vue-router";
import { TriangleAlert } from "lucide-vue-next";
import Button from "./ui/Button.vue";
import { store } from "../store";
import { api } from "../api";
import { fmtDuration, fmtTemp, fmtRpm, levelColor } from "../format";

const router = useRouter();
const active = computed(() => store.snapshot?.anomalies.filter((a) => !a.ignored) ?? []);
const main = computed(() => active.value[0]);
const related = computed(() => active.value.find((a) => a.relatedName)?.relatedName ?? null);

function ignore() {
  api.ignoreAnomalies();
}
</script>

<template>
  <div
    v-if="main && store.snapshot"
    class="flex items-start gap-3 rounded-lg border p-3.5"
    :style="{
      borderColor: `color-mix(in srgb, ${levelColor[main.level]} 45%, transparent)`,
      background: `color-mix(in srgb, ${levelColor[main.level]} 9%, var(--card))`,
    }"
  >
    <TriangleAlert class="mt-0.5 h-5 w-5 shrink-0" :style="{ color: levelColor[main.level] }" />
    <div class="min-w-0 flex-1">
      <div class="flex flex-wrap items-center gap-2 text-sm font-semibold">
        发现{{ main.title }}
        <span v-if="active.length > 1" class="text-xs font-normal text-muted-foreground">等 {{ active.length }} 项异常</span>
      </div>
      <div class="mt-1.5 flex flex-wrap gap-x-5 gap-y-1 text-xs text-muted-foreground tabular">
        <span>CPU 使用率：<b class="text-foreground">{{ store.snapshot.cpu.usage.toFixed(0) }}%</b></span>
        <span>CPU 温度：<b class="text-foreground">{{ fmtTemp(store.snapshot.cpu.temp) }}</b></span>
        <span>风扇转速：<b class="text-foreground">{{ fmtRpm(store.snapshot.fanRpm) }}</b></span>
        <span>异常持续时间：<b class="text-foreground">{{ fmtDuration(main.durationSecs) }}</b></span>
        <span v-if="related">主要关联进程：<b class="text-foreground">{{ related }}</b></span>
      </div>
    </div>
    <div class="flex shrink-0 gap-2">
      <Button size="sm" variant="secondary" @click="router.push({ path: '/processes', query: { filter: 'high' } })">查看异常进程</Button>
      <Button size="sm" @click="router.push({ path: '/diagnosis' })">立即诊断</Button>
      <Button size="sm" variant="ghost" @click="ignore">忽略此次提示</Button>
    </div>
  </div>
</template>
