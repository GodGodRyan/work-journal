<script setup lang="ts">
/**
 * 媒体附件块 NodeView（VueNodeViewRenderer 渲染，替代编辑态 DOM）：
 * - 按 media_type 渲染图片/视频/音频 + 文件名 caption
 * - 右下角手柄拖拽调整显示宽度（松开持久化到后端）
 * - 选中时高亮，并预取 data URL 进复制缓存（Ctrl+C 同步写剪贴板需要）
 * - 附件元数据通过 provide/inject（editor-attachments）从 RecordEditor 获取；
 *   附件未加载时降级为占位符文本
 */
import { computed, inject, ref, watch, type Ref } from "vue";
import { nodeViewProps, NodeViewWrapper } from "@tiptap/vue-3";
import type { Attachment } from "@/types/work-record";
import * as api from "@/api/records";
import { toast } from "@/components/ui/toast";
import { errMsg } from "@/lib/utils";
import { prefetchAttachCopy } from "./attach-copy-cache";

/** Tiptap NodeView 完整 props（editor/node/decorations/selected/extension/getPos/…） */
const props = defineProps(nodeViewProps);

/** 附件显示宽度钳制范围（px，与后端 MIN/MAX_ATTACH_WIDTH 一致） */
const ATTACH_MIN_WIDTH = 120;
const ATTACH_MAX_WIDTH = 2000;

/** 当前记录附件列表（RecordEditor provide） */
const attachments = inject<Ref<Attachment[]> | null>("editor-attachments", null);

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
</script>

<template>
  <!-- 附件未加载：降级为占位符文本（切换记录瞬间/附件已删除时） -->
  <NodeViewWrapper
    v-if="!att"
    as="span"
    class="text-muted-foreground"
    :data-attach-id="String(node.attrs.id ?? '')"
  >
    [附件:{{ node.attrs.id }}]
  </NodeViewWrapper>

  <!-- 媒体块独占一行（block 布局）：inline-block 并排时相邻 atom 之间没有合法文本光标位置
       （TextSelection 要求父节点含 inlineContent），点击图间/方向键会把光标强制挪到
       最近文本块，表现为光标错乱 -->
  <NodeViewWrapper
    v-else
    as="figure"
    :data-attach-id="String(att.id)"
    class="group relative my-1.5 block max-w-full rounded-md"
    :class="selected ? 'ring-2 ring-primary/50' : ''"
  >
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
  </NodeViewWrapper>
</template>
