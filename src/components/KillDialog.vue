<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { ShieldAlert, ShieldX, TriangleAlert, GitBranch, Info } from "lucide-vue-next";
import Dialog from "./ui/Dialog.vue";
import Button from "./ui/Button.vue";
import { api } from "../api";
import { store, toast, startObservation, procKey } from "../store";
import { fmtBytes } from "../format";
import type { KillMode, KillPreview, KillResult, ProcessInfo } from "../types";

type Phase = "loading" | "blocked" | "confirm" | "confirm2" | "running" | "result";

const phase = ref<Phase>("loading");
const preview = ref<KillPreview | null>(null);
const result = ref<KillResult | null>(null);
const mode = ref<KillMode>("graceful");
const proc = ref<ProcessInfo | null>(null);
const error = ref("");

const open = computed(() => !!store.kill);

const modeLabel = computed(
  () => ({ graceful: "结束进程", force: "强制结束进程", tree: "结束进程树" })[mode.value],
);

/** 关闭确认框；未成功结束时，若是从进程详情发起的，则恢复详情窗口 */
function close(restore = true) {
  const back = store.kill?.returnTo ?? null;
  store.kill = null;
  if (restore && back && !result.value?.success) store.detail = back;
}

watch(
  () => store.kill,
  async (k) => {
    if (!k) return;
    proc.value = k.proc;
    mode.value = k.mode;
    phase.value = "loading";
    preview.value = null;
    result.value = null;
    error.value = "";
    try {
      const p = await api.previewKill(k.proc.pid, k.proc.createTime);
      preview.value = p;
      if (!p.allowed) {
        phase.value = "blocked";
        return;
      }
      // 正常结束且关闭了确认、无警告时直接执行；强制结束与结束进程树始终需要确认
      if (k.mode === "graceful" && store.settings && !store.settings.confirmKill && p.warning === "none" && p.hasWindows) {
        await run();
        return;
      }
      phase.value = "confirm";
    } catch (e) {
      error.value = String(e);
      phase.value = "blocked";
    }
  },
);

async function run() {
  if (!proc.value) return;
  phase.value = "running";
  try {
    const p = proc.value;
    const r = await api.kill(p.pid, p.createTime, mode.value);
    result.value = r;
    if (r.success) {
      for (const pid of r.killed) {
        const found = store.snapshot?.processes.find((x) => x.pid === pid);
        if (found) store.removed.add(procKey(found));
      }
      startObservation(p);
      if (r.failed.length === 0) {
        toast({ kind: "success", title: `${modeLabel.value}成功`, message: `${r.message}。将持续监控 CPU 与温度变化。` });
        close(false);
        return;
      }
    }
    phase.value = "result";
  } catch (e) {
    result.value = {
      success: false,
      message: String(e),
      requiresAdmin: false,
      canForce: false,
      killed: [],
      failed: [],
      processName: proc.value.name,
    };
    phase.value = "result";
  }
}

function confirmFirst() {
  if (mode.value === "tree") {
    phase.value = "confirm2";
  } else {
    run();
  }
}

function switchToForce() {
  mode.value = "force";
  phase.value = "confirm";
}

async function relaunch() {
  try {
    await api.relaunchAdmin();
  } catch (e) {
    toast({ kind: "error", title: "未能以管理员身份重启", message: String(e) });
  }
}
</script>

<template>
  <Dialog :open="open" :title="modeLabel" width="480px" :z="60" :closable="phase !== 'running'" @close="close()">
    <div class="space-y-3 p-4 text-[13px]">
      <div v-if="phase === 'loading'" class="py-6 text-center text-muted-foreground">正在检查进程…</div>

      <!-- 受保护 -->
      <template v-else-if="phase === 'blocked'">
        <div class="flex gap-3 rounded-md border border-danger/40 bg-danger/10 p-3">
          <ShieldX class="h-5 w-5 shrink-0 text-danger" />
          <div>
            <div class="font-medium text-danger">无法执行该操作</div>
            <div class="mt-1 text-muted-foreground">{{ preview?.blockedReason || error }}</div>
          </div>
        </div>
        <div class="text-xs text-muted-foreground">FanGuard 不允许通过结束系统进程来尝试直接控制风扇。</div>
      </template>

      <!-- 确认 -->
      <template v-else-if="(phase === 'confirm' || phase === 'running') && proc">
        <div>确定{{ modeLabel }}？</div>
        <div class="grid grid-cols-[80px_1fr] gap-y-1.5 rounded-md bg-muted p-3 tabular">
          <span class="text-muted-foreground">进程名称：</span><span class="font-medium selectable">{{ proc.name }}</span>
          <span class="text-muted-foreground">PID：</span><span>{{ proc.pid }}</span>
          <span class="text-muted-foreground">CPU 占用：</span><span>{{ proc.cpu.toFixed(1) }}%</span>
          <span class="text-muted-foreground">内存占用：</span><span>{{ fmtBytes(proc.memory) }}</span>
          <template v-if="proc.gpu !== null">
            <span class="text-muted-foreground">GPU 占用：</span><span>{{ proc.gpu.toFixed(1) }}%</span>
          </template>
        </div>
        <div v-if="preview?.warningText" class="flex gap-2.5 rounded-md border border-warn/40 bg-warn/10 p-3 text-xs">
          <ShieldAlert class="h-4 w-4 shrink-0 text-warn" />
          <span>{{ preview.warningText }}</span>
        </div>
        <div v-if="mode === 'graceful' && preview && !preview.hasWindows" class="flex gap-2.5 rounded-md border border-border bg-muted p-3 text-xs">
          <Info class="h-4 w-4 shrink-0 text-primary" />
          <span>该进程没有可见窗口（后台进程），无法通过正常方式请求其退出，建议使用「强制结束进程」。</span>
        </div>
        <div v-if="mode === 'force'" class="flex gap-2.5 rounded-md border border-warn/40 bg-warn/10 p-3 text-xs">
          <TriangleAlert class="h-4 w-4 shrink-0 text-warn" />
          <span>强制结束不会给程序保存数据的机会，未保存的内容可能丢失。</span>
        </div>
        <div v-if="mode === 'tree'" class="rounded-md border border-border p-3 text-xs">
          <div class="mb-1.5 flex items-center gap-1.5 font-medium">
            <GitBranch class="h-3.5 w-3.5" /> 将同时强制结束 {{ preview?.children.length ?? 0 }} 个子进程
          </div>
          <div class="max-h-28 space-y-0.5 overflow-auto text-muted-foreground">
            <div v-for="c in preview?.children" :key="c.pid" class="flex justify-between">
              <span>{{ c.name }}</span>
              <span>PID {{ c.pid }}<span v-if="c.protected" class="ml-1 text-danger">（受保护，将跳过）</span></span>
            </div>
            <div v-if="!preview?.children.length">没有子进程</div>
          </div>
        </div>
        <div v-if="!store.elevated" class="text-xs text-muted-foreground">提示：当前未以管理员身份运行，结束部分系统或其他用户的进程可能需要管理员权限。</div>
      </template>

      <!-- 进程树二次确认 -->
      <template v-else-if="phase === 'confirm2' && proc">
        <div class="flex gap-3 rounded-md border border-danger/40 bg-danger/10 p-3">
          <TriangleAlert class="h-5 w-5 shrink-0 text-danger" />
          <div>
            <div class="font-medium">请再次确认</div>
            <div class="mt-1 text-muted-foreground">
              将强制结束 <b class="text-foreground">{{ proc.name }}</b>（PID {{ proc.pid }}）及其全部
              {{ preview?.children.length ?? 0 }} 个子进程，此操作无法撤销。
            </div>
          </div>
        </div>
      </template>

      <!-- 结果 -->
      <template v-else-if="phase === 'result' && result">
        <div
          class="rounded-md border p-3"
          :class="result.success ? 'border-ok/40 bg-ok/10' : 'border-danger/40 bg-danger/10'"
        >
          <div class="font-medium" :class="result.success ? 'text-ok' : 'text-danger'">
            {{ result.success ? "部分完成" : "操作未成功" }}
          </div>
          <div class="mt-1 text-muted-foreground selectable">{{ result.message }}</div>
        </div>
        <div v-if="result.failed.length" class="max-h-36 overflow-auto rounded-md border border-border p-2 text-xs">
          <div v-for="f in result.failed" :key="f.pid" class="py-0.5">
            <span class="font-medium">{{ f.name }}</span> (PID {{ f.pid }})：<span class="text-muted-foreground">{{ f.reason }}</span>
          </div>
        </div>
        <div v-if="result.requiresAdmin && !store.elevated" class="text-xs text-muted-foreground">
          该操作需要管理员权限。FanGuard 默认不申请管理员权限，如需继续，可以管理员身份重新启动 FanGuard。
        </div>
      </template>
    </div>

    <template #footer>
      <template v-if="phase === 'blocked'">
        <Button variant="secondary" @click="close()">知道了</Button>
      </template>
      <template v-else-if="phase === 'confirm' || phase === 'running'">
        <Button variant="secondary" :disabled="phase === 'running'" @click="close()">取消</Button>
        <Button
          v-if="mode === 'graceful' && preview && !preview.hasWindows"
          variant="destructive"
          @click="switchToForce"
          >强制结束进程</Button
        >
        <Button v-else :variant="mode === 'graceful' ? 'warning' : 'destructive'" :loading="phase === 'running'" @click="confirmFirst">
          {{ modeLabel }}
        </Button>
      </template>
      <template v-else-if="phase === 'confirm2'">
        <Button variant="secondary" @click="close()">取消</Button>
        <Button variant="destructive" @click="run">确认结束进程树</Button>
      </template>
      <template v-else-if="phase === 'result' && result">
        <Button v-if="result.requiresAdmin && !store.elevated" variant="outline" @click="relaunch">以管理员身份重启</Button>
        <Button v-if="result.canForce" variant="destructive" @click="switchToForce">强制结束进程</Button>
        <Button variant="secondary" @click="close()">关闭</Button>
      </template>
    </template>
  </Dialog>
</template>
