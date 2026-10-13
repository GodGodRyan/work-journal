<script setup lang="ts">
/**
 * 侧边栏置顶区（位于日历上方）：固定列出全部置顶记录，跨日期可见。
 * 点击记录跳转到所属日期并选中编辑；右键弹出菜单移入回收站（确认在主页完成）；
 * 无置顶记录时整个区块不渲染。
 */
import { onBeforeUnmount, onMounted, reactive } from "vue";
import { Pin, Trash2 } from "@lucide/vue";
import { useAppStore } from "@/stores/app";
import { fmtDateCn } from "@/composables/useDate";
import type { WorkRecord } from "@/types/work-record";

const store = useAppStore();

const emit = defineEmits<{
  select: [r: WorkRecord];
  delete: [r: WorkRecord];
}>();

// ---- 右键删除菜单（与 RecordCard 同模式：Teleport 到 body，点击任意处/Escape/滚动关闭）----
const menu = reactive({
  visible: false,
  x: 0,
  y: 0,
  record: null as WorkRecord | null,
});

function openMenu(e: MouseEvent, r: WorkRecord): void {
  closeMenu();
  menu.record = r;
  // 钳制在视口内（菜单约 128×36）
  menu.x = Math.min(e.clientX, window.innerWidth - 140);
  menu.y = Math.min(e.clientY, window.innerHeight - 44);
  menu.visible = true;
}

function closeMenu(): void {
  menu.visible = false;
}

function onMenuDelete(): void {
  const r = menu.record;
  closeMenu();
  if (r) emit("delete", r);
}

function onKeydown(e: KeyboardEvent): void {
  if (e.key === "Escape") closeMenu();
}

// 全局关闭：点击任意处 / 右键其它位置（捕获阶段，先关旧菜单）/ 滚动 / Escape
onMounted(() => {
  window.addEventListener("click", closeMenu);
  window.addEventListener("contextmenu", closeMenu, true);
  window.addEventListener("scroll", closeMenu, true);
  window.addEventListener("keydown", onKeydown);
});

onBeforeUnmount(() => {
  window.removeEventListener("click", closeMenu);
  window.removeEventListener("contextmenu", closeMenu, true);
  window.removeEventListener("scroll", closeMenu, true);
  window.removeEventListener("keydown", onKeydown);
});
</script>

<template>
  <div v-if="store.pinnedRecords.length > 0" class="rounded-md border">
    <div class="flex items-center gap-1.5 border-b px-3 py-2">
      <Pin class="h-4 w-4 shrink-0 text-primary" />
      <span class="text-[13px] font-medium">置顶</span>
      <span class="text-xs text-muted-foreground">{{ store.pinnedRecords.length }} 条</span>
    </div>
    <ul class="max-h-56 overflow-y-auto p-1">
      <li v-for="r in store.pinnedRecords" :key="r.id">
        <button
          type="button"
          data-ctx-menu
          class="flex w-full items-baseline gap-1 rounded-md px-2 py-2 text-left text-[13px] transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          :title="`${r.title || '（无标题）'} · ${fmtDateCn(r.record_date)}`"
          @click="emit('select', r)"
          @contextmenu.prevent="openMenu($event, r)"
        >
          <span class="min-w-0 flex-1 truncate">
            {{ r.title || "（无标题）" }}
          </span>
          <span class="shrink-0 text-[11px] tabular-nums text-muted-foreground">
            {{ fmtDateCn(r.record_date) }}
          </span>
        </button>
      </li>
    </ul>

    <!-- 右键菜单（移入回收站） -->
    <Teleport to="body">
      <div
        v-if="menu.visible"
        :style="{ left: `${menu.x}px`, top: `${menu.y}px` }"
        class="fixed z-50 w-32 rounded-md border bg-popover p-1 text-popover-foreground shadow-md"
      >
        <button
          type="button"
          class="flex w-full items-center gap-2 rounded-sm px-2 py-1.5 text-left text-xs transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          @click="onMenuDelete"
        >
          <Trash2 class="h-3.5 w-3.5" />
          移入回收站
        </button>
      </div>
    </Teleport>
  </div>
</template>
