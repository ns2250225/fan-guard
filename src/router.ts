import { createRouter, createWebHashHistory } from "vue-router";
import Dashboard from "./views/Dashboard.vue";

export const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", component: Dashboard },
    { path: "/processes", component: () => import("./views/Processes.vue") },
    { path: "/diagnosis", component: () => import("./views/Diagnosis.vue") },
    { path: "/hardware", component: () => import("./views/Hardware.vue") },
    { path: "/history", component: () => import("./views/History.vue") },
    { path: "/settings", component: () => import("./views/Settings.vue") },
  ],
});
