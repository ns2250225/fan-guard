import { reactive } from "vue";
import type { ProcessInfo } from "./types";

export const menuState = reactive({
  proc: null as ProcessInfo | null,
  x: 0,
  y: 0,
});

export function openMenu(proc: ProcessInfo, e: MouseEvent) {
  e.preventDefault();
  e.stopPropagation();
  menuState.proc = proc;
  // 避免菜单超出窗口
  const w = 210;
  const h = 260;
  menuState.x = Math.min(e.clientX, window.innerWidth - w - 8);
  menuState.y = Math.min(e.clientY, window.innerHeight - h - 8);
}

export function closeMenu() {
  menuState.proc = null;
}
