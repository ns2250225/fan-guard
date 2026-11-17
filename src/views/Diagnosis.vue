<script setup lang="ts">
import { computed, onActivated, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { RefreshCw, Thermometer, Fan, Cpu, MonitorSmartphone, Lightbulb, Info, HardDrive, Eye, CircleStop, ShieldCheck, EyeOff, Clock } from "lucide-vue-next";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Badge from "../components/ui/Badge.vue";
import ScoreBars from "../components/ScoreBars.vue";
import Observations from "../components/Observations.vue";
import { api } from "../api";
import { store, openDetail, openKill, findProcess, toggleWhitelist, toast } from "../store";
import { fmtBytes, fmtDuration, fmtRpm, fmtTemp, fmtTime, fmtWatt, gradeColor, levelColor } from "../format";
import type { DiagnosisReport, Suspect } from "../types";

const route = useRoute();
const router = useRouter();
const report = ref<DiagnosisReport | null>(null);
const loading = ref(false);
let timer: number | undefined;

async function run(manual = false) {
  loading.value = manual;
  try {
    report.value = await api.diagnose();
    if (manual) toast({ kind: "info", title: "诊断完成", message: report.value.title });
  } catch (e) {
    if (manual) toast({ kind: "error", title: "诊断失败", message: String(e) });
  } finally {
    loading.value = false;
  }
}

onMounted(() => {
  run(route.query.action === "diagnose");
  timer = window.setInterval(() => run(false), 3000);
});
onBeforeUnmount(() => clearInterval(timer));
watch(
  () => route.query.action,
  (a) => {
    if (a === "diagnose") run(true);
  },
);

const r = computed(() => report.value);
const color = computed(() => levelColor[r.value?.level ?? "normal"]);

function view(s: Suspect) {
  openDetail(s.pid, s.createTime);
}
function kill(s: Suspect) {
  const p = findProcess(s.pid, s.createTime);
  if (!p) {
    toast({ kind: "warning", title: "进程已经退出" });
    return;
  }
  openKill(p, "graceful");
}
async function ignore(code?: string) {
  await api.ignoreAnomalies(code);
  toast({ kind: "info", title: "已忽略", message: "该异常结束前不再提示，相关事件仍会记录在历史中" });
  run(false);
}
function jumpSuggestion(pid: number | null) {
  if (pid === null) return;
  const p = findProcess(pid);
  if (p) openDetail(p.pid, p.createTime);
}
</script>

<template>
  <div class="h-full overflow-auto">
    <div class="mx-auto max-w-[1400px] space-y-4 p-5">
      <div class="flex items-center justify-between">
        <div>
          <h1 class="text-lg font-semibold">智能诊断</h1>
          <div class="text-xs text-muted-foreground">
            持续分析 CPU / GPU / 温度 / 功耗 / 风扇与进程负载的关联 · 自动每 3 秒更新
            <template v-if="r"> · 上次诊断 {{ fmtTime(r.generatedAt, false) }}</template>
          </div>
        </div>
        <Button :loading="loading" @click="run(true)"><RefreshCw class="h-3.5 w-3.5" /> 立即诊断</Button>
      </div>

      <div v-if="!r" class="py-20 text-center text-muted-foreground">正在分析…</div>
      <template v-else>
        <!-- 诊断结论 -->
        <div
          class="rounded-lg border p-4"
          :style="{ borderColor: `color-mix(in srgb, ${color} 45%, transparent)`, background: `color-mix(in srgb, ${color} 7%, var(--card))` }"
        >
          <div class="flex items-start justify-between gap-6">
            <div>
              <div class="text-base font-semibold">{{ r.title }}</div>
              <div class="mt-2 flex items-center gap-4 text-[13px]">
                <span>异常等级：<b :style="{ color }">{{ r.levelText }}</b></span>
                <span v-if="r.durationSecs !== null" class="flex items-center gap-1">
                  <Clock class="h-3.5 w-3.5 text-muted-foreground" /> 异常持续时间：<b>{{ fmtDuration(r.durationSecs) }}</b>
                </span>
                <span class="text-muted-foreground">统计窗口：最近 {{ r.windowSecs }} 秒</span>
              </div>
            </div>
            <div class="grid shrink-0 grid-cols-3 gap-x-6 gap-y-2 text-xs tabular">
              <div class="flex items-center gap-1.5"><Cpu class="h-3.5 w-3.5 text-muted-foreground" /> CPU <b>{{ r.hardware.cpu.toFixed(0) }}%</b></div>
              <div class="flex items-center gap-1.5">
                <Thermometer class="h-3.5 w-3.5 text-muted-foreground" /> CPU 温度 <b>{{ fmtTemp(r.hardware.cpuTemp) }}</b>
              </div>
              <div class="flex items-center gap-1.5"><Fan class="h-3.5 w-3.5 text-muted-foreground" /> 风扇 <b>{{ fmtRpm(r.hardware.fan) }}</b></div>
              <div class="flex items-center gap-1.5">
                <MonitorSmartphone class="h-3.5 w-3.5 text-muted-foreground" /> GPU <b>{{ r.hardware.gpu === null ? "不可用" : r.hardware.gpu.toFixed(0) + "%" }}</b>
              </div>
              <div class="flex items-center gap-1.5">
                <Thermometer class="h-3.5 w-3.5 text-muted-foreground" /> GPU 温度 <b>{{ fmtTemp(r.hardware.gpuTemp) }}</b>
              </div>
              <div class="flex items-center gap-1.5">CPU 功耗 <b>{{ fmtWatt(r.hardware.cpuPower) }}</b></div>
              <div class="flex items-center gap-1.5">
                <HardDrive class="h-3.5 w-3.5 text-muted-foreground" /> 磁盘温度 <b>{{ fmtTemp(r.hardware.diskTemp) }}</b>
              </div>
              <div class="flex items-center gap-1.5">
                磁盘活动 <b>{{ r.hardware.diskActive === null ? "-" : r.hardware.diskActive.toFixed(0) + "%" }}</b>
              </div>
            </div>
          </div>

          <div v-if="r.anomalies.length" class="mt-4 space-y-1.5">
            <div
              v-for="a in r.anomalies"
              :key="a.code"
              class="flex items-center gap-3 rounded-md bg-card/70 px-3 py-2 text-xs"
              :class="a.ignored ? 'opacity-60' : ''"
            >
              <span class="h-2 w-2 shrink-0 rounded-full" :style="{ background: levelColor[a.level] }" />
              <span class="w-40 shrink-0 font-medium">{{ a.title }}</span>
              <span class="min-w-0 flex-1 text-muted-foreground">{{ a.description }}</span>
              <span class="shrink-0 tabular text-muted-foreground">已持续 {{ fmtDuration(a.durationSecs) }}</span>
              <Badge v-if="a.ignored" color="var(--muted-foreground)">已忽略</Badge>
              <Button v-else size="xs" variant="ghost" @click="ignore(a.code)"><EyeOff class="h-3 w-3" /> 忽略</Button>
            </div>
          </div>
        </div>

        <Observations />

        <div class="grid grid-cols-[1fr_380px] gap-4">
          <!-- 主要排查对象 -->
          <Card title="主要排查对象" subtitle="按异常关联分数排序（白名单进程排在后面，但不会隐藏）">
            <div v-if="!r.suspects.length" class="py-10 text-center text-sm text-muted-foreground">
              <ShieldCheck class="mx-auto mb-2 h-8 w-8 text-ok" />
              没有发现需要重点排查的进程
            </div>
            <div class="space-y-3">
              <div
                v-for="(s, i) in r.suspects"
                :key="s.pid + ':' + s.createTime"
                class="rounded-md border border-border p-3"
                :class="s.whitelisted ? 'opacity-70' : ''"
              >
                <div class="flex items-start justify-between gap-4">
                  <div class="min-w-0">
                    <div class="flex items-center gap-2">
                      <span class="text-xs text-muted-foreground">{{ i + 1 }}.</span>
                      <span class="font-semibold">{{ s.name }}</span>
                      <span class="text-xs text-muted-foreground">PID {{ s.pid }}</span>
                      <Badge :color="gradeColor(s.score)">{{ s.gradeText }}</Badge>
                      <Badge v-if="s.protected" color="var(--ok)">受保护</Badge>
                      <Badge v-if="s.whitelisted" color="var(--primary)">白名单</Badge>
                    </div>
                    <div class="mt-1.5 grid grid-cols-5 gap-x-4 text-xs tabular">
                      <span><span class="text-muted-foreground">CPU：</span>{{ s.cpuAvg.toFixed(1) }}%</span>
                      <span><span class="text-muted-foreground">内存：</span>{{ fmtBytes(s.memory) }}</span>
                      <span><span class="text-muted-foreground">GPU：</span>{{ s.gpuAvg === null ? "不可用" : s.gpuAvg.toFixed(1) + "%" }}</span>
                      <span><span class="text-muted-foreground">持续时间：</span>{{ s.durationSecs }} 秒</span>
                      <span><span class="text-muted-foreground">关联程度：</span><b :style="{ color: gradeColor(s.score) }">{{ s.correlation }}</b></span>
                    </div>
                  </div>
                  <div class="text-right">
                    <div class="text-2xl font-semibold tabular" :style="{ color: gradeColor(s.score) }">{{ s.score.toFixed(0) }}</div>
                    <div class="text-[11px] text-muted-foreground">关联分数</div>
                  </div>
                </div>
                <div class="mt-3 grid grid-cols-2 gap-4">
                  <ScoreBars :b="s.breakdown" />
                  <ul class="list-disc space-y-0.5 pl-4 text-xs text-muted-foreground">
                    <li v-for="reason in s.reasons" :key="reason">{{ reason }}</li>
                  </ul>
                </div>
                <div class="mt-3 flex gap-2">
                  <Button size="sm" variant="secondary" @click="view(s)"><Eye class="h-3.5 w-3.5" /> 查看进程</Button>
                  <Button v-if="!s.protected" size="sm" variant="warning" @click="kill(s)"><CircleStop class="h-3.5 w-3.5" /> 结束进程</Button>
                  <Button size="sm" variant="ghost" @click="toggleWhitelist(s.name)">
                    {{ s.whitelisted ? "移出白名单" : "忽略（加入白名单）" }}
                  </Button>
                </div>
              </div>
            </div>
          </Card>

          <div class="space-y-4">
            <Card title="诊断理由">
              <ul class="space-y-1.5 text-xs">
                <li v-for="x in r.reasons" :key="x" class="flex gap-2">
                  <span class="mt-1.5 h-1 w-1 shrink-0 rounded-full bg-muted-foreground" />
                  <span>{{ x }}</span>
                </li>
              </ul>
            </Card>

            <Card title="建议操作">
              <div class="space-y-2.5">
                <div v-for="sg in r.suggestions" :key="sg.title" class="rounded-md bg-muted p-2.5 text-xs">
                  <div class="flex items-center gap-1.5 font-medium"><Lightbulb class="h-3.5 w-3.5 text-notice" /> {{ sg.title }}</div>
                  <div class="mt-1 text-muted-foreground">{{ sg.detail }}</div>
                  <button v-if="sg.relatedPid !== null" class="mt-1.5 font-medium text-primary hover:underline" @click="jumpSuggestion(sg.relatedPid)">
                    跳转到 {{ sg.relatedName }} →
                  </button>
                </div>
              </div>
              <div v-if="r.anomalies.some((a) => !a.ignored)" class="mt-3 flex gap-2">
                <Button size="sm" variant="secondary" @click="router.push({ path: '/processes', query: { filter: 'high' } })">查看高负载进程</Button>
                <Button size="sm" variant="ghost" @click="ignore()">忽略全部</Button>
              </div>
            </Card>

            <Card v-if="r.dataNotes.length" title="数据说明">
              <ul class="space-y-1.5 text-xs text-muted-foreground">
                <li v-for="n in r.dataNotes" :key="n" class="flex gap-2"><Info class="mt-0.5 h-3.5 w-3.5 shrink-0" /> {{ n }}</li>
              </ul>
            </Card>

            <Card title="评分说明">
              <div class="space-y-1 text-xs text-muted-foreground">
                <div>CPU 使用率 35% · GPU 使用率 25% · 内存占用及变化 10% · CPU/GPU 功耗关联 20% · 持续高负载时间 10%</div>
                <div class="flex flex-wrap gap-2 pt-1">
                  <Badge color="var(--ok)">0–39 正常</Badge>
                  <Badge color="var(--notice)">40–59 需要关注</Badge>
                  <Badge color="var(--warn)">60–79 高负载</Badge>
                  <Badge color="var(--danger)">80–100 重点排查</Badge>
                </div>
                <div class="pt-1">基于时间窗口统计而非瞬时值；分数为优先排查分数，不是真实风扇因果贡献率。数据不可用的维度会移除并按比例重新分配权重。</div>
              </div>
            </Card>
          </div>
        </div>
      </template>
    </div>
  </div>
</template>
