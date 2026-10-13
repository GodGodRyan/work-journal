<script setup lang="ts">
import { onBeforeUnmount, onMounted } from "vue";
import { useRouter } from "vue-router";
import AppNav from "@/components/AppNav.vue";
import { Toaster } from "@/components/ui/toast";
import { useAppStore } from "@/stores/app";
import { useHotkeys } from "@/composables/useHotkeys";

const store = useAppStore();
const router = useRouter();

store.initTheme();

// 全局快捷键：通过 store 信号驱动对应页面动作
useHotkeys({
  onNew: async () => {
    if (router.currentRoute.value.path !== "/") await router.push("/");
    store.requestNewRecord();
  },
  onSearch: async () => {
    if (router.currentRoute.value.path !== "/search") await router.push("/search");
    store.requestSearchFocus();
  },
  onSave: () => store.requestManualSave(),
  onSettings: async () => {
    if (router.currentRoute.value.path !== "/settings") await router.push("/settings");
  },
});

// 全局禁止右键默认行为：仅记录卡片（data-ctx-menu 标记）允许右键弹出删除菜单，
// 其余区域（日历/编辑器/侧边栏等）右键一律无响应。
// 捕获阶段先于卡片自身的 contextmenu 处理器执行，放行后由卡片弹菜单。
function blockContextMenu(e: MouseEvent): void {
  const t = e.target as Element | null;
  if (t?.closest?.("[data-ctx-menu]")) return;
  e.preventDefault();
}

// 全局禁用浏览器特征操作，让应用呈现原生桌面程序体验：
// - 拖放：全局 preventDefault 屏蔽默认行为（浏览器打开文件/元素拖出）；
//   编辑器区域自己挂监听处理文件拖放（见 RecordEditor.onCreate），
//   全局拦截只 preventDefault 不 stopPropagation，不影响其处理
// - 刷新/查找/缩放等浏览器快捷键（Cmd/Ctrl+R、F5、Cmd/Ctrl+F/U/P、Cmd/Ctrl+/-/0）
// - 中键点击（浏览器的自动滚动/新标签行为）
function blockDragOver(e: DragEvent): void {
  e.preventDefault();
}

function blockDragStart(e: DragEvent): void {
  e.preventDefault();
}

function blockBrowserKeys(e: KeyboardEvent): void {
  const k = e.key.toLowerCase();
  if (k === "f5") {
    e.preventDefault();
    return;
  }
  if (
    (e.metaKey || e.ctrlKey) &&
    ["r", "f", "u", "p", "=", "+", "-", "0"].includes(k)
  ) {
    e.preventDefault();
  }
}

function blockMiddleClick(e: MouseEvent): void {
  if (e.button === 1) e.preventDefault();
}

onMounted(() => {
  window.addEventListener("contextmenu", blockContextMenu, true);
  window.addEventListener("dragover", blockDragOver, true);
  window.addEventListener("drop", blockDragOver, true);
  window.addEventListener("dragstart", blockDragStart, true);
  window.addEventListener("keydown", blockBrowserKeys, true);
  window.addEventListener("mousedown", blockMiddleClick, true);
});

onBeforeUnmount(() => {
  window.removeEventListener("contextmenu", blockContextMenu, true);
  window.removeEventListener("dragover", blockDragOver, true);
  window.removeEventListener("drop", blockDragOver, true);
  window.removeEventListener("dragstart", blockDragStart, true);
  window.removeEventListener("keydown", blockBrowserKeys, true);
  window.removeEventListener("mousedown", blockMiddleClick, true);
});
</script>

<template>
  <div class="flex h-screen w-screen overflow-hidden bg-background text-foreground">
    <AppNav />
    <main class="min-w-0 flex-1 overflow-hidden">
      <RouterView />
    </main>
    <Toaster />
  </div>
</template>
