<script setup lang="ts">
import { computed, reactive } from "vue";
import { X, Plus, ShieldAlert } from "lucide-vue-next";
import Card from "../components/ui/Card.vue";
import Button from "../components/ui/Button.vue";
import Switch from "../components/ui/Switch.vue";
import Select from "../components/ui/Select.vue";
import Segmented from "../components/ui/Segmented.vue";
import { store, saveSettings, toast } from "../store";
import { api } from "../api";
import type { Settings } from "../types";

const st = computed(() => store.settings!);

function set<K extends keyof Settings>(key: K, value: Settings[K]) {
  saveSettings({ [key]: value } as Partial<Settings>);
}

function num<K extends keyof Settings>(key: K, e: Event) {
  const v = Number((e.target as HTMLInputElement).value);
  if (!isNaN(v)) set(key, v as Settings[K]);
}

const procNames = computed(() => {
  const names = new Set((store.snapshot?.processes ?? []).map((p) => p.name));
  return [...names].sort((a, b) => a.localeCompare(b));
});

const inputs = reactive({ whitelist: "", ignoreList: "" });

function addTo(list: "whitelist" | "ignoreList") {
  const name = inputs[list].trim();
  if (!name) return;
  if (st.value[list].some((n) => n.toLowerCase() === name.toLowerCase())) {
    toast({ kind: "info", title: "已存在", message: name });
    return;
  }
  set(list, [...st.value[list], name]);
  inputs[list] = "";
}
function removeFrom(list: "whitelist" | "ignoreList", name: string) {
  set(
    list,
    st.value[list].filter((n) => n !== name),
  );
}

async function relaunch() {
  try {
    await api.relaunchAdmin();
  } catch (e) {
    toast({ kind: "error", title: "未能以管理员身份重启", message: String(e) });
  }
}

const intervalOpts = [
  { label: "0.5 秒", value: 500 },
  { label: "1 秒", value: 1000 },
  { label: "2 秒", value: 2000 },
  { label: "5 秒", value: 5000 },
];
const sensorOpts = [
  { label: "1 秒", value: 1000 },
  { label: "2 秒", value: 2000 },
  { label: "3 秒", value: 3000 },
  { label: "5 秒", value: 5000 },
  { label: "10 秒", value: 10000 },
];
const historyOpts = [
  { label: "1 秒", value: 1000 },
  { label: "5 秒", value: 5000 },
  { label: "10 秒", value: 10000 },
  { label: "30 秒", value: 30000 },
  { label: "60 秒", value: 60000 },
];
const retentionOpts = [
  { label: "1 天", value: 1 },
  { label: "3 天", value: 3 },
  { label: "7 天", value: 7 },
  { label: "14 天", value: 14 },
  { label: "30 天", value: 30 },
];
</script>

<template>
  <div v-if="st" class="h-full overflow-auto">
    <div class="mx-auto max-w-[900px] space-y-4 p-5">
      <div>
        <h1 class="text-lg font-semibold">系统设置</h1>
        <div class="text-xs text-muted-foreground">修改后自动保存</div>
      </div>

      <!-- 基础设置 -->
      <Card title="基础设置">
        <div class="divide-y divide-border">
          <div class="flex items-center justify-between py-3">
            <div><div>开机自动启动</div><div class="text-xs text-muted-foreground">登录 Windows 后在后台启动并最小化到托盘</div></div>
            <Switch :model-value="st.autostart" @update:model-value="(v) => set('autostart', v)" />
          </div>
          <div class="flex items-center justify-between py-3">
            <div><div>关闭窗口时最小化到托盘</div><div class="text-xs text-muted-foreground">关闭后在系统托盘继续监控，右键托盘图标可退出</div></div>
            <Switch :model-value="st.minimizeToTray" @update:model-value="(v) => set('minimizeToTray', v)" />
          </div>
          <div class="flex items-center justify-between py-3">
            <div><div>显示桌面通知</div><div class="text-xs text-muted-foreground">检测到持续高负载、高温或风扇异常时通知</div></div>
            <Switch :model-value="st.notifications" @update:model-value="(v) => set('notifications', v)" />
          </div>
          <div class="flex items-center justify-between py-3">
            <div><div>自动启动硬件监控</div><div class="text-xs text-muted-foreground">关闭后软件启动时监控处于暂停状态</div></div>
            <Switch :model-value="st.autoStartMonitoring" @update:model-value="(v) => set('autoStartMonitoring', v)" />
          </div>
          <div class="flex items-center justify-between py-3">
            <div>主题</div>
            <Segmented
              :model-value="st.theme"
              :options="[
                { label: '深色', value: 'dark' },
                { label: '浅色', value: 'light' },
                { label: '跟随系统', value: 'system' },
              ]"
              @update:model-value="(v) => set('theme', v as Settings['theme'])"
            />
          </div>
        </div>
      </Card>

      <!-- 监控设置 -->
      <Card title="监控设置" subtitle="阈值为初始建议值，可根据硬件厂商提供的温度限制进行调整">
        <div class="divide-y divide-border">
          <div class="flex items-center justify-between py-3">
            <div>进程刷新间隔</div>
            <Select :model-value="st.processIntervalMs" :options="intervalOpts" @update:model-value="(v) => set('processIntervalMs', v)" />
          </div>
          <div class="flex items-center justify-between py-3">
            <div>
              <div>传感器数据来源</div>
              <div class="text-xs text-muted-foreground">自动：HWiNFO 可用时优先使用，缺失项由 LibreHardwareMonitor 补充；HWiNFO 需开启「共享内存支持」</div>
            </div>
            <Select
              :model-value="st.sensorSource"
              :options="[
                { label: '自动', value: 'auto' },
                { label: '仅 HWiNFO', value: 'hwinfo' },
                { label: '仅 LibreHardwareMonitor', value: 'lhm' },
              ]"
              @update:model-value="(v) => set('sensorSource', v)"
            />
          </div>
          <div class="flex items-center justify-between py-3">
            <div>
              <div>自动集成 HWiNFO</div>
              <div class="text-xs text-muted-foreground">自动导入「下载」文件夹中的 HWiNFO 便携版，自动开启共享内存、后台启动，并在免费版 12 小时到期时自动重启</div>
            </div>
            <Switch :model-value="st.hwinfoAuto" @update:model-value="(v) => set('hwinfoAuto', v)" />
          </div>
          <div class="flex items-center justify-between py-3">
            <div><div>硬件传感器刷新间隔</div><div class="text-xs text-muted-foreground">温度、功耗、风扇转速</div></div>
            <Select :model-value="st.sensorIntervalMs" :options="sensorOpts" @update:model-value="(v) => set('sensorIntervalMs', v)" />
          </div>
          <div class="flex items-center justify-between py-3">
            <div>历史数据采样间隔</div>
            <Select :model-value="st.historyIntervalMs" :options="historyOpts" @update:model-value="(v) => set('historyIntervalMs', v)" />
          </div>
          <div class="flex items-center justify-between py-3">
            <div><div>历史数据保留时间</div><div class="text-xs text-muted-foreground">过期数据会被定期清理</div></div>
            <Select :model-value="st.historyRetentionDays" :options="retentionOpts" @update:model-value="(v) => set('historyRetentionDays', v)" />
          </div>
          <div v-for="f in [
              { key: 'cpuThreshold', label: 'CPU 高负载阈值', unit: '%', min: 10, max: 100, step: 1, hint: '持续高于该值时分析 CPU 高负载进程' },
              { key: 'gpuThreshold', label: 'GPU 高负载阈值', unit: '%', min: 10, max: 100, step: 1, hint: '持续高于该值时分析 GPU 高负载进程' },
              { key: 'cpuTempThreshold', label: 'CPU 温度告警阈值', unit: '°C', min: 40, max: 110, step: 1, hint: '' },
              { key: 'gpuTempThreshold', label: 'GPU 温度告警阈值', unit: '°C', min: 40, max: 110, step: 1, hint: '' },
              { key: 'diskTempThreshold', label: '磁盘温度告警阈值', unit: '°C', min: 35, max: 90, step: 1, hint: '机械硬盘建议 50–55°C，固态硬盘建议 60–70°C' },
              { key: 'fanRpmThreshold', label: '风扇转速告警阈值', unit: 'RPM', min: 500, max: 20000, step: 100, hint: '仅在硬件支持读取 RPM 时生效' },
              { key: 'sustainSeconds', label: '持续判定时间', unit: '秒', min: 5, max: 600, step: 5, hint: '负载需持续超过阈值该时长才判定为异常，避免瞬时波动误报' },
            ] as const" :key="f.key" class="flex items-center justify-between py-3">
            <div><div>{{ f.label }}</div><div v-if="f.hint" class="text-xs text-muted-foreground">{{ f.hint }}</div></div>
            <div class="flex items-center gap-2">
              <input
                type="number"
                :min="f.min"
                :max="f.max"
                :step="f.step"
                :value="st[f.key]"
                class="h-8 w-24 rounded-md border border-border bg-card px-2 text-right text-[13px] tabular outline-none focus:ring-2 focus:ring-primary/40"
                @change="num(f.key, $event)"
              />
              <span class="w-8 text-xs text-muted-foreground">{{ f.unit }}</span>
            </div>
          </div>
        </div>
      </Card>

      <!-- 进程设置 -->
      <Card title="进程设置">
        <div class="divide-y divide-border">
          <div class="flex items-center justify-between py-3">
            <div><div>显示系统进程</div><div class="text-xs text-muted-foreground">关键系统进程与 Windows 系统组件</div></div>
            <Switch :model-value="st.showSystemProcesses" @update:model-value="(v) => set('showSystemProcesses', v)" />
          </div>
          <div class="flex items-center justify-between py-3">
            <div><div>显示后台服务</div><div class="text-xs text-muted-foreground">运行在 Session 0 的系统服务进程</div></div>
            <Switch :model-value="st.showServices" @update:model-value="(v) => set('showServices', v)" />
          </div>
          <div class="flex items-center justify-between py-3">
            <div><div>启用结束进程确认</div><div class="text-xs text-muted-foreground">强制结束、结束进程树以及系统/服务进程始终需要确认</div></div>
            <Switch :model-value="st.confirmKill" @update:model-value="(v) => set('confirmKill', v)" />
          </div>

          <div class="py-3">
            <div>进程白名单</div>
            <div class="mb-2 text-xs text-muted-foreground">白名单只影响告警和诊断排序（排在后面），不会隐藏系统安全相关信息</div>
            <div class="mb-2 flex flex-wrap gap-1.5">
              <span v-for="n in st.whitelist" :key="n" class="inline-flex items-center gap-1 rounded-md bg-accent px-2 py-1 text-xs">
                {{ n }}
                <button class="text-muted-foreground hover:text-danger" @click="removeFrom('whitelist', n)"><X class="h-3 w-3" /></button>
              </span>
              <span v-if="!st.whitelist.length" class="text-xs text-muted-foreground">暂无</span>
            </div>
            <div class="flex gap-2">
              <input
                v-model="inputs.whitelist"
                list="proc-names"
                placeholder="输入进程名称，例如 Code.exe"
                class="h-8 flex-1 rounded-md border border-border bg-card px-2.5 text-[13px] outline-none focus:ring-2 focus:ring-primary/40"
                @keydown.enter="addTo('whitelist')"
              />
              <Button variant="secondary" @click="addTo('whitelist')"><Plus class="h-3.5 w-3.5" /> 添加</Button>
            </div>
          </div>

          <div class="py-3">
            <div>自动忽略指定进程</div>
            <div class="mb-2 text-xs text-muted-foreground">被忽略的进程不参与异常诊断与告警，但仍显示在进程列表中</div>
            <div class="mb-2 flex flex-wrap gap-1.5">
              <span v-for="n in st.ignoreList" :key="n" class="inline-flex items-center gap-1 rounded-md bg-accent px-2 py-1 text-xs">
                {{ n }}
                <button class="text-muted-foreground hover:text-danger" @click="removeFrom('ignoreList', n)"><X class="h-3 w-3" /></button>
              </span>
              <span v-if="!st.ignoreList.length" class="text-xs text-muted-foreground">暂无</span>
            </div>
            <div class="flex gap-2">
              <input
                v-model="inputs.ignoreList"
                list="proc-names"
                placeholder="输入进程名称"
                class="h-8 flex-1 rounded-md border border-border bg-card px-2.5 text-[13px] outline-none focus:ring-2 focus:ring-primary/40"
                @keydown.enter="addTo('ignoreList')"
              />
              <Button variant="secondary" @click="addTo('ignoreList')"><Plus class="h-3.5 w-3.5" /> 添加</Button>
            </div>
          </div>
          <datalist id="proc-names">
            <option v-for="n in procNames" :key="n" :value="n" />
          </datalist>
        </div>
      </Card>

      <!-- 权限与关于 -->
      <Card title="权限与隐私">
        <div class="space-y-3 text-xs text-muted-foreground">
          <div class="flex items-center justify-between">
            <div>
              <div class="text-[13px] text-foreground">当前权限：{{ store.elevated ? "管理员" : "标准用户" }}</div>
              <div>FanGuard 默认不申请管理员权限。以管理员身份运行可读取更多 CPU 温度、功耗与主板风扇传感器，并结束需要更高权限的进程。</div>
            </div>
            <Button v-if="!store.elevated" size="sm" variant="outline" @click="relaunch"><ShieldAlert class="h-3.5 w-3.5" /> 以管理员身份重启</Button>
          </div>
          <div class="flex items-center justify-between">
            <div>
              <div class="text-[13px] text-foreground">始终以管理员身份运行</div>
              <div>启动时请求一次 UAC 授权。笔记本嵌入式控制器的风扇转速、CPU 温度与功耗需要此权限；拒绝授权时以普通权限继续运行。</div>
            </div>
            <Switch :model-value="st.runAsAdmin" @update:model-value="(v) => set('runAsAdmin', v)" />
          </div>
          <div>所有进程信息与硬件数据仅保存在本机，不会上传；不收集用户文件内容；所有操作记录保存在本地数据库。</div>
          <div>V1.0 不修改风扇转速、不提供风扇曲线控制，也不会自动结束任何进程。</div>
        </div>
      </Card>
    </div>
  </div>
</template>
