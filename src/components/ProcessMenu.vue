<script setup lang="ts">
import { onBeforeUnmount, onMounted } from "vue";
import { CircleStop, Zap, GitBranch, FolderOpen, Info, ShieldCheck, ShieldOff } from "lucide-vue-next";
import { menuState, closeMenu } from "../menu";
import { openDetail, openKill, toggleWhitelist, isWhitelisted, toast } from "../store";
import { api } from "../api";
import type { KillMode } from "../types";

function onDoc() {
  closeMenu();
}
onMounted(() => {
  window.addEventListener("mousedown", onDoc);
  window.addEventListener("blur", onDoc);
  window.addEventListener("resize", onDoc);
});
onBeforeUnmount(() => {
  window.removeEventListener("mousedown", onDoc);
  window.removeEventListener("blur", onDoc);
  window.removeEventListener("resize", onDoc);
});

function kill(mode: KillMode) {
  const p = menuState.proc;
  closeMenu();
  if (p) openKill(p, mode);
}

async function openLocation() {
  const p = menuState.proc;
  closeMenu();
  if (!p?.path) {
    toast({ kind: "warning", title: "无法获取文件路径", message: "该进程受系统保护或需要管理员权限才能读取路径" });
    return;
  }
  try {
    await api.openFileLocation(p.path);
  } catch (e) {
    toast({ kind: "error", title: "打开文件位置失败", message: String(e) });
  }
}

function detail() {
  const p = menuState.proc;
  closeMenu();
  if (p) openDetail(p.pid, p.createTime);
}

function whitelist() {
  const p = menuState.proc;
  closeMenu();
  if (p) toggleWhitelist(p.name);
}
</script>

<template>
  <Teleport to="body">
    <div
      v-if="menuState.proc"
      class="fixed z-[55] w-[210px] overflow-hidden rounded-md border border-border bg-popover py-1 text-[13px] shadow-xl"
      :style="{ left: `${menuState.x}px`, top: `${menuState.y}px` }"
      @mousedown.stop
    >
      <div class="truncate px-3 py-1.5 text-xs text-muted-foreground">{{ menuState.proc.name }} · PID {{ menuState.proc.pid }}</div>
      <template v-if="menuState.proc.kind !== 'critical'">
        <button class="flex h-8 w-full items-center gap-2.5 px-3 hover:bg-accent" @click="kill('graceful')">
          <CircleStop class="h-4 w-4 text-warn" /> 结束进程
        </button>
        <button class="flex h-8 w-full items-center gap-2.5 px-3 hover:bg-accent" @click="kill('force')">
          <Zap class="h-4 w-4 text-danger" /> 强制结束进程
        </button>
        <button class="flex h-8 w-full items-center gap-2.5 px-3 hover:bg-accent" @click="kill('tree')">
          <GitBranch class="h-4 w-4 text-danger" /> 结束进程树
        </button>
      </template>
      <div v-else class="flex h-8 items-center gap-2.5 px-3 text-muted-foreground">
        <ShieldCheck class="h-4 w-4 text-ok" /> 关键系统进程（受保护）
      </div>
      <div class="my-1 h-px bg-border" />
      <button class="flex h-8 w-full items-center gap-2.5 px-3 hover:bg-accent" @click="openLocation">
        <FolderOpen class="h-4 w-4" /> 打开文件位置
      </button>
      <button class="flex h-8 w-full items-center gap-2.5 px-3 hover:bg-accent" @click="detail">
        <Info class="h-4 w-4" /> 查看详细信息
      </button>
      <button class="flex h-8 w-full items-center gap-2.5 px-3 hover:bg-accent" @click="whitelist">
        <component :is="isWhitelisted(menuState.proc.name) ? ShieldOff : ShieldCheck" class="h-4 w-4" />
        {{ isWhitelisted(menuState.proc.name) ? "从白名单移除" : "添加到白名单" }}
      </button>
    </div>
  </Teleport>
</template>
