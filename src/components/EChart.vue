<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, shallowRef, watch } from "vue";
import * as echarts from "echarts/core";
import { LineChart, BarChart } from "echarts/charts";
import {
  GridComponent,
  TooltipComponent,
  LegendComponent,
  MarkLineComponent,
  MarkPointComponent,
  MarkAreaComponent,
  DataZoomComponent,
} from "echarts/components";
import { CanvasRenderer } from "echarts/renderers";

echarts.use([
  LineChart,
  BarChart,
  GridComponent,
  TooltipComponent,
  LegendComponent,
  MarkLineComponent,
  MarkPointComponent,
  MarkAreaComponent,
  DataZoomComponent,
  CanvasRenderer,
]);

const props = withDefaults(defineProps<{ option: any; height?: string; notMerge?: boolean }>(), {
  height: "240px",
  notMerge: false,
});
const emit = defineEmits<{ (e: "click", params: any): void }>();

const el = ref<HTMLDivElement>();
const chart = shallowRef<echarts.ECharts>();
let ro: ResizeObserver | null = null;

function render() {
  if (!chart.value || !props.option) return;
  chart.value.setOption(props.option, { notMerge: props.notMerge, lazyUpdate: true, replaceMerge: ["series", "yAxis"] });
}

function onTheme() {
  // 主题切换后重新渲染，让坐标轴颜色从 CSS 变量重新读取
  if (!chart.value) return;
  chart.value.clear();
  render();
}

onMounted(() => {
  chart.value = echarts.init(el.value!, undefined, { renderer: "canvas" });
  chart.value.on("click", (p: any) => emit("click", p));
  render();
  ro = new ResizeObserver(() => chart.value?.resize());
  ro.observe(el.value!);
  window.addEventListener("fg-theme", onTheme);
});

watch(() => props.option, render);

onBeforeUnmount(() => {
  ro?.disconnect();
  window.removeEventListener("fg-theme", onTheme);
  chart.value?.dispose();
});
</script>

<template>
  <div ref="el" :style="{ height, width: '100%' }" />
</template>
