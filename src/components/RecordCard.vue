<script setup lang="ts">
/**
 * 主页记录列表项：标题 + 修改时间 + 2 行内容预览。
 * 置顶记录显示 Pin 标记与日期徽标（置顶跨日期，需要区分归属），
 * 卡片常显置顶/取消置顶按钮；右键弹出删除菜单（移入回收站）。
 */
import { onBeforeUnmount, onMounted, reactive } from "vue";
import { Pin, Trash2 } from "@lucide/vue";
import { cn, maskAttachmentMarkers } from "@/lib/utils";
import { fmtDateCn, fmtTime } from "@/composables/useDate";
import type { WorkRecord } from "@/types/work-record";

const props = defineProps<{
  record: WorkRecord;
  selected: boolean;
}>();

const emit = defineEmits<{
  select: [];
  togglePinned: [];
  requestDelete: [];
}>();

/** 内容预览：与内容区保持一致换行（渲染侧 whitespace-pre-line），
 *  连续空行折叠为单个，并只保留前 2 行。
 *  注意 line-clamp 与 whitespace-pre-line 组合会失效，行数限制靠数据层截断，
 *  CSS 层用 max-h-8 overflow-hidden 兜底超长行自动换行溢出 */
function previewText(r: WorkRecord): string {
  return maskAttachmentMarkers(r.content_text)
    .replace(/\n{2,}/g, "\n")
    .split("\n")
    .slice(0, 2)
    .join("\n");
}

// ---- 右键删除菜单 ----
const menu = reactive({ visible: false, x: 0, y: 0 });

function openMenu(e: MouseEvent): void {
  closeMenu();
  // 钳制在视口内（菜单约 128×36）
  menu.x = Math.min(e.clientX, window.innerWidth - 140);
  menu.y = Math.min(e.clientY, window.innerHeight - 44);
  menu.visible = true;
}

function closeMenu(): void {
  menu.visible = false;
}

function onMenuDelete(): void {
  closeMenu();
  emit("requestDelete");
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
  <!-- data-ctx-menu：全局右键屏蔽的白名单标记（App.vue 捕获阶段校验），仅卡片允许右键 -->
  <div
    data-ctx-menu
    :class="cn(
      'group cursor-pointer rounded-md border p-3 transition-colors',
      props.selected
        ? 'border-primary/60 bg-accent'
        : 'border-transparent hover:bg-accent/60',
    )"
    @click="emit('select')"
    @contextmenu.prevent="openMenu"
  >
    <div class="flex items-start gap-2">
      <div class="min-w-0 flex-1">
        <div class="flex items-baseline justify-between gap-2">
          <p class="flex min-w-0 items-center gap-1 truncate text-sm font-medium">
            <!-- 置顶标记（Pin 图标承担置顶语义，不依赖颜色） -->
            <Pin v-if="props.record.is_pinned" class="h-3.5 w-3.5 shrink-0 text-primary" />
            <span class="truncate">{{ props.record.title || "（无标题）" }}</span>
            <!-- 置顶记录跨日期显示，需要日期徽标区分归属 -->
            <span
              v-if="props.record.is_pinned"
              class="shrink-0 rounded bg-accent px-1 text-[10px] font-normal text-muted-foreground"
            >
              {{ fmtDateCn(props.record.record_date) }}
            </span>
          </p>
          <span class="flex shrink-0 items-center gap-1">
            <span class="text-[11px] tabular-nums text-muted-foreground">
              {{ fmtTime(props.record.updated_at) }}
            </span>
            <!-- 置顶/取消置顶（常显，点击不触发选中） -->
            <button
              type="button"
              class="rounded p-0.5 transition-colors hover:bg-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
              :class="
                props.record.is_pinned
                  ? 'text-primary'
                  : 'text-muted-foreground/60 hover:text-primary'
              "
              :aria-label="props.record.is_pinned ? '取消置顶' : '置顶'"
              :title="props.record.is_pinned ? '取消置顶' : '置顶'"
              @click.stop="emit('togglePinned')"
            >
              <Pin
                :class="cn('h-3.5 w-3.5', props.record.is_pinned && 'fill-primary text-primary')"
              />
            </button>
          </span>
        </div>
        <p
          v-if="props.record.content_text"
          class="mt-0.5 max-h-8 overflow-hidden whitespace-pre-line text-xs text-muted-foreground"
        >
          {{ previewText(props.record) }}
        </p>
      </div>
    </div>

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
