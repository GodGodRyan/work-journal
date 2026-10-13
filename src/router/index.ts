import { createRouter, createWebHashHistory } from "vue-router";

/**
 * hash 模式：Tauri 本地文件环境无需服务端路由支持。
 * 五个页面：主页（日期编辑）/ 时间线 / 搜索 / 回收站 / 设置。
 */
const router = createRouter({
  history: createWebHashHistory(),
  routes: [
    { path: "/", name: "home", component: () => import("@/views/HomeView.vue") },
    { path: "/timeline", name: "timeline", component: () => import("@/views/TimelineView.vue") },
    { path: "/search", name: "search", component: () => import("@/views/SearchView.vue") },
    { path: "/trash", name: "trash", component: () => import("@/views/TrashView.vue") },
    { path: "/settings", name: "settings", component: () => import("@/views/SettingsView.vue") },
  ],
});

export default router;
