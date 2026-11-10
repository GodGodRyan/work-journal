<script setup lang="ts">
/**
 * 媒体附件块 NodeView（VueNodeViewRenderer 渲染，替代编辑态 DOM）：
 * - 按 media_type 渲染图片/视频/音频 + 文件名 caption
 * - hover 时右上角浮出操作条：复制（写剪贴板）/ 导出（保存对话框）/ 删除（从文档移除节点）
 * - 右下角手柄拖拽调整显示宽度（松开持久化到后端）
 * - 选中时高亮，并预取 data URL 进复制缓存（Ctrl+C 同步写剪贴板需要）
 * - 附件元数据通过 provide/inject（editor-attachments）从 RichTextEditor 获取；
 *   附件未加载时降级为占位符文本
 */
import { computed, inject, ref, watch, type Ref } from "vue";
import { nodeViewProps, NodeViewWrapper } from "@tiptap/vue-3";
import { Copy, Download, Trash2 } from "@lucide/vue";
import type { Attachment } from "@/types/work-record";
import * as api from "@/api/records";
import { toast } from "@/components/ui/toast";
import { errMsg } from "@/lib/utils";
import { prefetchAttachCopy } from "./attach-copy-cache";

/** Tiptap NodeView 完整 props（editor/node/decorations/selected/extension/getPos/deleteNode/…） */
const props = defineProps(nodeViewProps);

/** 附件显示宽度钳制范围（px，与后端 MIN/MAX_ATTACH_WIDTH 一致） */
const ATTACH_MIN_WIDTH = 120;
const ATTACH_MAX_WIDTH = 2000;

/** 当前记录附件列表（RichTextEditor provide） */
const attachments = inject<Ref<Attachment[]> | null>("editor-attachments", null);

/** 复制媒体块（RichTextEditor provide：构造节点选区写剪贴板 HTML + 文件） */
const copyAttachment = inject<((id: number | null) => void) | null>("attachment-copy", null);

const att = computed<Attachment | null>(() => {
  const id = props.node.attrs.id as number | null;
  return id != null ? (attachments?.value.find((a) => a.id === id) ?? null) : null;
});

const url = computed(() => (att.value ? api.attachmentUrl(att.value.path) : ""));

/** 当前显示宽度（px）；null = 默认尺寸 */
const displayWidth = ref<number | null>(null);
watch(
  att,
  (a) => {
    displayWidth.value = a?.width != null ? clampAttachWidth(a.width) : null;
  },
  { immediate: true },
);

// 选中时预取复制数据（附件加载完成后再预取一次，覆盖「先选中后加载」的场景）
watch(
  () => [props.selected, att.value?.id] as const,
  ([selected]) => {
    if (selected && att.value) void prefetchAttachCopy(att.value);
  },
);

/** 钳制附件显示宽度：限制在 [最小, 最大] 与编辑器可用宽度内 */
function clampAttachWidth(w: number): number {
  const editorW = props.editor?.view.dom.clientWidth ?? ATTACH_MAX_WIDTH;
  const maxW = Math.max(ATTACH_MIN_WIDTH, Math.min(ATTACH_MAX_WIDTH, editorW - 32));
  return Math.min(maxW, Math.max(ATTACH_MIN_WIDTH, Math.round(w)));
}

/** 拖拽调宽：pointer 捕获实时预览，松开后持久化 */
function onHandlePointerDown(e: PointerEvent): void {
  if (!att.value) return;
  e.preventDefault();
  e.stopPropagation();
  const handle = e.currentTarget as HTMLElement;
  const startX = e.clientX;
  const startW =
    displayWidth.value ?? handle.parentElement?.querySelector("img,video,audio")?.getBoundingClientRect().width ?? 300;
  const onMove = (ev: PointerEvent): void => {
    displayWidth.value = clampAttachWidth(startW + (ev.clientX - startX));
  };
  const onUp = (): void => {
    handle.removeEventListener("pointermove", onMove);
    handle.removeEventListener("pointerup", onUp);
    handle.removeEventListener("pointercancel", onUp);
    void saveWidth(att.value!.id, displayWidth.value ?? startW);
  };
  handle.setPointerCapture(e.pointerId);
  handle.addEventListener("pointermove", onMove);
  handle.addEventListener("pointerup", onUp);
  handle.addEventListener("pointercancel", onUp);
}

/** 附件宽度持久化（失败仅 toast 提示，不打断编辑） */
async function saveWidth(id: number, width: number): Promise<void> {
  try {
    await api.setAttachmentWidth(id, width);
    const a = attachments?.value.find((x) => x.id === id);
    if (a) a.width = width;
  } catch (e) {
    toast({ title: "保存附件大小失败", description: errMsg(e), variant: "destructive" });
  }
}

// ---- 操作条 ----

function onCopy(): void {
  copyAttachment?.(att.value?.id ?? null);
}

/** 导出附件：保存对话框选路径，后端拷贝文件；取消对话框不提示 */
async function onExport(): Promise<void> {
  const id = att.value?.id;
  if (id == null) return;
  try {
    const path = await api.exportAttachment(id);
    if (path) toast({ title: "导出成功", description: `已导出到 ${path}` });
  } catch (e) {
    toast({ title: "导出失败", description: errMsg(e), variant: "destructive" });
  }
}

/** 删除媒体块：从文档移除节点；附件物理删除由 RichTextEditor 的孤儿清理机制处理 */
function onDelete(): void {
  props.deleteNode();
}
</script>

<template>
  <!-- 根元素恒定 figure[data-attach-id]：必须与 Attachment 扩展的 parseHTML 规则一致，
       否则占位符 span 会触发 ProseMirror DOMObserver 重解析，原子节点被丢弃（媒体块消失） -->
  <NodeViewWrapper
    as="figure"
    :data-attach-id="String(node.attrs.id ?? '')"
    class="group relative my-1.5 block max-w-full rounded-md"
    :class="selected ? 'ring-2 ring-primary/50' : ''"
  >
    <!-- 附件未加载：降级为占位符文本（切换记录瞬间/附件已删除时） -->
    <template v-if="!att">
      <span class="text-muted-foreground">[附件:{{ node.attrs.id }}]</span>
    </template>

    <!-- 媒体块独占一行（block 布局）：inline-block 并排时相邻 atom 之间没有合法文本光标位置，
         点击图间/方向键会把光标强制挪到最近文本块，表现为光标错乱 -->
    <template v-else>
      <!-- hover 操作条：复制 / 导出 / 删除。mousedown.prevent 阻止 PM 选中节点，保持操作独立。
           z-20 高于右下角拖拽手柄（z-10），小图上两者可能重叠，操作按钮需优先接收指针事件 -->
      <div
        class="absolute right-1 top-1 z-20 flex items-center gap-0.5 rounded-md border bg-background/90 p-0.5 opacity-0 shadow-sm transition-opacity group-hover:opacity-100"
        @mousedown.prevent
      >
        <button
          type="button"
          class="rounded p-1 hover:bg-accent"
          title="复制媒体块"
          aria-label="复制媒体块"
          @click="onCopy"
        >
          <Copy class="h-3 w-3" />
        </button>
        <button
          type="button"
          class="rounded p-1 hover:bg-accent"
          title="导出媒体文件"
          aria-label="导出媒体文件"
          @click="onExport"
        >
          <Download class="h-3 w-3" />
        </button>
        <button
          type="button"
          class="rounded p-1 hover:bg-accent hover:text-destructive"
          title="删除媒体块"
          aria-label="删除媒体块"
          @click="onDelete"
        >
          <Trash2 class="h-3 w-3" />
        </button>
      </div>
      <div class="relative block max-w-full">
        <img
          v-if="att.media_type === 'image'"
          :src="url"
          :alt="att.file_name"
          :style="displayWidth != null ? { width: `${displayWidth}px` } : undefined"
          class="block max-h-72 max-w-full rounded-md border bg-muted object-contain"
        />
        <video
          v-else-if="att.media_type === 'video'"
          :src="url"
          controls
          preload="metadata"
          :style="displayWidth != null ? { width: `${displayWidth}px` } : undefined"
          class="block max-h-72 max-w-full rounded-md border bg-black"
        />
        <audio
          v-else
          :src="url"
          controls
          preload="metadata"
          :style="displayWidth != null ? { width: `${displayWidth}px` } : undefined"
          class="block w-80 max-w-full"
        />
        <!-- 右下角拖拽手柄：pointer 捕获实时调整宽度，松开后持久化 -->
        <span
          :aria-label="`调整附件大小 ${att.file_name}`"
          title="拖拽调整大小"
          class="absolute bottom-1 right-1 z-10 h-3.5 w-3.5 cursor-nwse-resize rounded-sm border border-muted-foreground/50 bg-background/80 opacity-0 transition-opacity group-hover:opacity-100"
          @pointerdown="onHandlePointerDown"
        />
      </div>
      <figcaption class="pt-1 text-[10px] text-muted-foreground">
        <span class="block max-w-[220px] truncate" :title="att.file_name">{{ att.file_name }}</span>
      </figcaption>
    </template>
  </NodeViewWrapper>
</template>
