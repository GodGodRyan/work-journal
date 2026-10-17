<script setup lang="ts">
/**
 * 记录编辑器：标题 / 标签 / 富文本内容（Tiptap）。
 * - 停止输入 500ms 自动保存（useAutoSave），保存按钮与 ⌘/Ctrl+S 手动兜底（跟随系统）
 * - 切换记录 / 组件卸载前先 flush 未落盘内容（通过 captured editingId 保存到正确的记录）
 * - 删除走软删除（AlertDialog 确认），数据可到回收站恢复
 * - 内容为 Tiptap 富文本：存储 HTML（<p>/<strong>/<span style>/<figure data-attach-id>），
 *   后端自动提取纯文本副本供 FTS 搜索/摘要/导出
 * - 粘贴的媒体异步保存为附件后插入媒体块（光标/选区由 ProseMirror 管理，不漂移）
 * - 媒体块退格/Delete 键整体删除，孤儿附件 800ms 防抖自动清理
 * - 选中媒体块 Cmd/Ctrl+C 复制（HTML + 文件双格式进剪贴板）
 * - 选中文字右键弹出格式菜单：加粗 / 字号 / 颜色
 */
import { computed, nextTick, onBeforeUnmount, onMounted, provide, reactive, ref, watch } from "vue";
import { Bold, Copy, Download, Loader2, NotebookPen, Pin, Plus, Save, Trash2, X } from "@lucide/vue";
import { EditorContent, useEditor } from "@tiptap/vue-3";
import StarterKit from "@tiptap/starter-kit";
import { Color, FontSize, TextStyle } from "@tiptap/extension-text-style";
import { NodeSelection, TextSelection } from "@tiptap/pm/state";
import { Fragment } from "@tiptap/pm/model";
import type { EditorView } from "@tiptap/pm/view";
import type { Editor, JSONContent } from "@tiptap/core";
import type { Node as ProseMirrorNode } from "@tiptap/pm/model";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { toast } from "@/components/ui/toast";
import * as api from "@/api/records";
import { useAutoSave } from "@/composables/useAutoSave";
import { modLabel } from "@/composables/useHotkeys";
import { useAppStore } from "@/stores/app";
import { errMsg, joinTags, splitTags } from "@/lib/utils";
import { fmtTime } from "@/composables/useDate";
import type { Attachment, WorkRecord } from "@/types/work-record";
import AttachmentExtension from "@/components/editor/Attachment";
import { attachCopyCache, dataUrlToFile } from "@/components/editor/attach-copy-cache";

const props = defineProps<{ record: WorkRecord | null }>();

const emit = defineEmits<{
  saved: [r: WorkRecord];
  deleted: [];
  create: [];
  togglePinned: [];
}>();

const store = useAppStore();

const form = reactive({
  title: "",
  content: "",
  status: "pending" as "pending" | "completed",
  tags: [] as string[],
});
const tagInput = ref("");
const deleting = ref(false);

// ---- 媒体附件 ----
/** 当前记录附件列表（MediaBlock NodeView 通过 inject 读取） */
const attachments = ref<Attachment[]>([]);
provide("editor-attachments", attachments);
/** 孤儿附件清理防抖句柄（连续输入只执行最后一次） */
let orphanCleanupTimer: ReturnType<typeof setTimeout> | null = null;
/** 孤儿附件待删集合：物理删除延迟到切换记录时（编辑会话内撤销可保住附件） */
const pendingOrphanIds = new Set<number>();

/** 当前正在编辑的记录 id（保存时使用，切换记录后仍能落盘到正确记录） */
let editingId: number | null = null;
/** 从 props 同步表单时置位，抑制变更监听 */
let syncing = false;

// ---- Tiptap 编辑器 ----

/** 右键菜单状态（format=选中文字的格式菜单；media=选中媒体块的删除菜单） */
const ctxMenu = ref<{ x: number; y: number; open: boolean; kind: "format" | "media" }>({
  x: 0,
  y: 0,
  open: false,
  kind: "format",
});
/** 字号选项（px） */
const FONT_SIZES = ["12px", "16px", "20px", "24px"];
/** 颜色色板 */
const COLOR_PALETTE = [
  "#1a1a1a",
  "#6b7280",
  "#e5484d",
  "#f76b15",
  "#f5c518",
  "#30a46c",
  "#0091ff",
  "#8e4ec6",
];

const editor = useEditor({
  extensions: [
    StarterKit.configure({
      // 仅保留正文所需：段落/文本/加粗/撤销重做；其余一律禁用
      heading: false,
      blockquote: false,
      bulletList: false,
      orderedList: false,
      listItem: false,
      listKeymap: false,
      code: false,
      codeBlock: false,
      horizontalRule: false,
      hardBreak: false,
      italic: false,
      strike: false,
      underline: false,
      link: false,
      trailingNode: false,
    }),
    TextStyle,
    FontSize,
    Color,
    AttachmentExtension,
  ],
  editorProps: {
    attributes: {
      class: "tiptap focus:outline-none",
      "data-gramm": "false",
    },
    handlePaste: onEditorPaste,
    handleDOMEvents: {
      copy: onEditorCopy,
    },
  },
  // 右键菜单不走 handleDOMEvents.contextmenu：
  // App.vue 在 window 捕获阶段对编辑器区域右键 preventDefault，
  // 而 ProseMirror 的 eventBelongsToView 检查 event.defaultPrevented，
  // 命中后 runCustomHandler 被短路，handleDOMEvents 处理器永远不会执行。
  // 因此改为直接在编辑器 DOM 上手动挂监听（preventDefault 不阻止事件传播）。
  onCreate: ({ editor: ed }) => {
    ed.view.dom.addEventListener("contextmenu", (event: MouseEvent) => {
      onEditorContextMenu(ed.view, event);
    });
    // 文件拖放：与右键同理，App.vue 在 window 捕获阶段对 dragover/drop preventDefault
    // （PM 的 drop 内置处理器同样会被 defaultPrevented 短路），因此手动挂监听自己处理。
    // dragover 必须 preventDefault 才会触发 drop（App.vue 已全局取消，此处兜底）。
    const dom = ed.view.dom;
    let dragDepth = 0;
    dom.addEventListener("dragenter", (event: DragEvent) => {
      if (!event.dataTransfer?.types.includes("Files")) return;
      dragDepth++;
      dom.classList.add("drag-over");
    });
    dom.addEventListener("dragleave", () => {
      dragDepth = Math.max(0, dragDepth - 1);
      if (dragDepth === 0) dom.classList.remove("drag-over");
    });
    dom.addEventListener("dragover", (event: DragEvent) => {
      event.preventDefault();
      if (event.dataTransfer) event.dataTransfer.dropEffect = "copy";
    });
    dom.addEventListener("drop", (event: DragEvent) => {
      dragDepth = 0;
      dom.classList.remove("drag-over");
      onEditorDrop(ed.view, event);
    });
  },
  onUpdate: ({ editor: ed }) => {
    const html = ed.getHTML();
    if (html !== form.content) form.content = html;
    scheduleOrphanCleanup();
  },
});

/** 当前选区的格式状态（右键菜单高亮用） */
const ctxBold = computed(() => editor.value?.isActive("bold") ?? false);
const ctxFontSize = computed<string | null>(
  () => (editor.value?.getAttributes("textStyle").fontSize as string | undefined) ?? null,
);
const ctxColor = computed<string | null>(
  () => (editor.value?.getAttributes("textStyle").color as string | undefined) ?? null,
);

// ---- 自动保存 ----

const autosave = useAutoSave({
  save: async () => {
    if (editingId == null) return;
    const updated = await api.updateRecord(
      editingId,
      form.title,
      form.content,
      form.status,
      joinTags(form.tags),
    );
    emit("saved", updated);
  },
  onError: (e) => {
    toast({ title: "自动保存失败", description: errMsg(e), variant: "destructive" });
  },
});

// 切换记录：先保存上一条，再同步新记录表单
watch(
  () => props.record?.id,
  async (id, oldId) => {
    if (oldId != null && oldId !== id) {
      const ok = await autosave.flush();
      // 保存成功后才物理删除旧记录的孤儿附件；失败则放弃删除（宁可残留不误删）
      if (ok) {
        await flushPendingOrphans();
      } else {
        pendingOrphanIds.clear();
      }
    }
    syncing = true;
    form.title = props.record?.title ?? "";
    form.content = props.record?.content ?? "";
    form.status = props.record?.status ?? "pending";
    form.tags = splitTags(props.record?.tags ?? "");
    tagInput.value = "";
    editingId = id ?? null;
    // form 的深层 watch 在赋值后的下一轮 flush 才执行，syncing 须保持到那之后再复位：
    // 同步复位会让「载入赋值」被误判为「用户编辑」，打开记录就触发一次无意义自动保存
    await nextTick();
    syncing = false;
    // 丢弃上一条记录遗留的孤儿清理任务（附件列表即将清空，避免误删）
    if (orphanCleanupTimer != null) {
      clearTimeout(orphanCleanupTimer);
      orphanCleanupTimer = null;
    }
    attachments.value = [];
    editor.value?.commands.setContent(
      separateAdjacentAttachments(htmlForContent(form.content)),
      { emitUpdate: false },
    );
    // replaceWith 会把选区映射到文档末尾：末尾恰是媒体块（atom）时光标会落在节点内，
    // 被解释为 NodeSelection，导致切换记录后媒体块被自动选中（高亮 + 右键误弹删除菜单）
    editor.value?.commands.setTextSelection(0);
    void loadAttachments(id);
  },
  { immediate: true },
);

// 表单变更 → 触发防抖自动保存
watch(
  form,
  () => {
    if (!syncing && props.record) autosave.markDirty();
  },
  { deep: true },
);

/** 手动保存（兜底） */
async function manualSave(): Promise<void> {
  if (!props.record) return;
  const ok = await autosave.flush();
  if (ok) {
    toast({
      title: "已保存",
      description: `保存于 ${new Date().toLocaleTimeString("zh-CN", {
        hour: "2-digit",
        minute: "2-digit",
      })}`,
    });
  }
}

/** 软删除（确认后执行） */
async function confirmDelete(): Promise<void> {
  if (!props.record) return;
  deleting.value = true;
  try {
    await api.softDelete(props.record.id);
    emit("deleted");
    toast({ title: "已移入回收站", description: "可在回收站页面恢复" });
  } catch (e) {
    toast({ title: "删除失败", description: errMsg(e), variant: "destructive" });
  } finally {
    deleting.value = false;
  }
}

// ---- 标签编辑 ----

function addTag(): void {
  const t = tagInput.value.trim().replace(/[,，;；]/g, "");
  if (!t) return;
  if (t.length > 50) {
    toast({ title: "标签过长", description: "单个标签不能超过 50 个字符", variant: "destructive" });
    return;
  }
  if (!form.tags.includes(t)) form.tags.push(t);
  tagInput.value = "";
}

function removeTag(t: string): void {
  form.tags = form.tags.filter((x) => x !== t);
}

function onTagKeydown(e: KeyboardEvent): void {
  if (e.key === "Enter" || e.key === ",") {
    e.preventDefault();
    addTag();
  } else if (e.key === "Backspace" && tagInput.value === "" && form.tags.length > 0) {
    form.tags = form.tags.slice(0, -1);
  }
}

// ---- 媒体附件 ----

/** 加载当前记录的附件列表（切换记录 / 添加成功后调用） */
async function loadAttachments(id: number | null | undefined): Promise<void> {
  if (id == null) {
    attachments.value = [];
    return;
  }
  try {
    attachments.value = await api.listAttachments(id);
  } catch (e) {
    attachments.value = [];
    toast({ title: "加载附件失败", description: errMsg(e), variant: "destructive" });
  }
}

/** MIME → 扩展名映射（与后端白名单对齐；仅白名单内类型可粘贴为附件） */
const MIME_EXT: Record<string, string> = {
  "image/png": "png",
  "image/jpeg": "jpg",
  "image/gif": "gif",
  "image/webp": "webp",
  "image/bmp": "bmp",
  "video/mp4": "mp4",
  "video/webm": "webm",
  "video/quicktime": "mov",
  "video/x-m4v": "m4v",
  "video/x-matroska": "mkv",
  "audio/mpeg": "mp3",
  "audio/wav": "wav",
  "audio/x-wav": "wav",
  "audio/mp4": "m4a",
  "audio/x-m4a": "m4a",
  "audio/aac": "aac",
  "audio/ogg": "ogg",
  "audio/flac": "flac",
};

/** 由文件名的扩展名推断类型（文件粘贴时 MIME 可能为空） */
function extFromName(name: string): string | null {
  const m = name.toLowerCase().match(/\.([a-z0-9]+)$/);
  if (!m) return null;
  return Object.values(MIME_EXT).includes(m[1]) ? m[1] : null;
}

function readAsDataURL(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const fr = new FileReader();
    fr.onload = () => resolve(fr.result as string);
    fr.onerror = () => reject(new Error("读取剪贴板内容失败"));
    fr.readAsDataURL(file);
  });
}

// ---- 粘贴 ----

/**
 * 在指定位置插入纯文本（按行拆为段落），并把光标落到插入内容末尾。
 * 不用 Tiptap insertContent 的 text 版：其底层是 tr.insertText，
 * 在非文本块位置（原子节点旁/文档末尾）会吞掉相邻节点。
 * 段落数组走 PM replaceWith 分支，插入 block 安全。
 */
function insertPlainTextAt(pos: number, text: string): void {
  const ed = editor.value;
  if (!ed) return;
  const lines = text.split(/\r?\n/);
  const $pos = ed.state.doc.resolve(pos);
  // 空段落内粘贴：文本直接填入（避免在空段后又叠一个新段）；
  // 多行时首行填入、其余行作为段落接在其后
  if ($pos.parent.isTextblock && $pos.parent.childCount === 0) {
    const tr = ed.state.tr.insertText(lines[0], pos, pos);
    let end = pos + lines[0].length;
    if (lines.length > 1) {
      const rest = lines.slice(1).map((line) =>
        ed.schema.nodes.paragraph.create(null, line ? ed.schema.text(line) : undefined),
      );
      const frag = Fragment.fromArray(rest);
      tr.replaceWith(pos + 1 + lines[0].length, pos + 1 + lines[0].length, frag);
      // 末尾位置用 near 吸附，避免落在段落闭合边界上（TextSelection.create 会失败）
      end = pos + 1 + lines[0].length + frag.size - 1;
    }
    tr.setSelection(TextSelection.near(tr.doc.resolve(end), 1));
    ed.view.dispatch(tr);
    ed.view.focus();
    return;
  }
  const nodes = lines.map((line) =>
    ed.schema.nodes.paragraph.create(null, line ? ed.schema.text(line) : undefined),
  );
  const frag = Fragment.fromArray(nodes);
  const tr = ed.state.tr.replaceWith(pos, pos, frag);
  // 选区移到插入内容末尾（后续媒体块应插在文本之后）
  tr.setSelection(TextSelection.create(tr.doc, pos + frag.size));
  ed.view.dispatch(tr);
  ed.view.focus();
}

/**
 * 内容区粘贴统一接管：
 * - 剪贴板 HTML 含 data-attach-id（编辑器内部复制的媒体块）：交给 Tiptap 默认解析，
 *   剪贴板中的 File 项忽略（避免附件重复入库）
 * - 无媒体文件：纯文本插入（防外部富文本样式混入）
 * - 有媒体文件：文本纯文本插入 + 文件逐个异步保存为附件后插入媒体块
 */
function onEditorPaste(_view: EditorView, event: ClipboardEvent): boolean {
  const ed = editor.value;
  if (!ed || !props.record) return false;
  const html = event.clipboardData?.getData("text/html") ?? "";
  const text = event.clipboardData?.getData("text/plain") ?? "";
  const items = Array.from(event.clipboardData?.items ?? []);
  const media = items.filter((i) => i.kind === "file");

  if (html.includes("data-attach-id")) return false; // 内部媒体块复制粘贴

  if (media.length === 0) {
    if (!text) return false;
    insertPlainTextAt(ed.state.selection.from, text);
    return true;
  }

  // 有媒体文件：完全接管。保存是异步的，先用快照记录插入位置与文档状态，
  // 插入时若文档未变化则用原位置（ProseMirror 选区不受 WebView selection 漂移影响）
  event.preventDefault();
  const savedPos = ed.state.selection.from;
  const docBefore = ed.state.doc;
  if (text) insertPlainTextAt(savedPos, text);
  const insertPos = text ? ed.state.selection.to : savedPos;

  const files: { file: File; ext: string }[] = [];
  for (const it of media) {
    const f = it.getAsFile();
    if (!f) continue;
    const ext = MIME_EXT[it.type] ?? extFromName(f.name);
    if (!ext) {
      toast({
        title: "无法识别的粘贴内容",
        description: f.name || it.type || "未知文件",
        variant: "destructive",
      });
      continue;
    }
    files.push({ file: f, ext });
  }
  if (files.length > 0) void savePastedFiles(files, insertPos, docBefore);
  return true;
}

/** 点击内容区空白（shell 内、ProseMirror 之外，如 padding 区域）时聚焦编辑器：
 *  点击 PM 内部由其自身处理光标定位，这里不干预。
 *  空白处光标一律落到文档末尾文本处（与浏览器点击 contenteditable 空白区行为一致）；
 *  不能只 focus() 保持原选区——若原选区是选中的媒体块（NodeSelection），
 *  图片保持高亮而打字插入别处，出现「光标错乱」。
 *  末尾是媒体块（atom）时先补一个空段落：atom 之后无文本位置，光标无处渲染。 */
function onShellMouseDown(e: MouseEvent): void {
  const ed = editor.value;
  if (!ed) return;
  const t = e.target as Node | null;
  if (t && ed.view.dom.contains(t)) return;
  // 阻止 mousedown 默认聚焦行为后显式聚焦，避免与浏览器默认焦点竞争
  e.preventDefault();
  const docSize = ed.state.doc.content.size;
  const $end = ed.state.doc.resolve(docSize);
  let tr = ed.state.tr;
  // 文档末尾位置的 parent 恒为 doc（非文本块），判断「末尾是否文本块」必须看 lastChild；
  // 否则每次点击空白都走补段落分支，每点一次多一个空行
  const last = ed.state.doc.lastChild;
  if (last && last.isTextblock) {
    tr = tr.setSelection(TextSelection.near($end));
  } else {
    tr = tr.insert(docSize, ed.schema.nodes.paragraph.create());
    tr.setSelection(TextSelection.create(tr.doc, docSize + 1));
  }
  ed.view.dispatch(tr);
  ed.view.focus();
}

/** 拖放文件到编辑器：白名单过滤后走与粘贴相同的附件入库流程 */
function onEditorDrop(_view: EditorView, event: DragEvent): void {
  const ed = editor.value;
  if (!ed || !props.record) return;
  event.preventDefault();
  const raw = Array.from(event.dataTransfer?.files ?? []);
  const files: { file: File; ext: string }[] = [];
  for (const f of raw) {
    const ext = MIME_EXT[f.type] ?? extFromName(f.name);
    if (!ext) {
      toast({
        title: "无法识别的文件",
        description: f.name || "未知文件",
        variant: "destructive",
      });
      continue;
    }
    files.push({ file: f, ext });
  }
  // 拖放后立即聚焦：savePastedFiles 仅在成功路径聚焦，
  // 这里覆盖全部无效文件与保存失败路径，保证拖放后可直接输入
  ed.view.focus();
  if (files.length === 0) return;
  const insertPos = resolveDropPos(ed, event.clientX, event.clientY);
  const docBefore = ed.state.doc;
  void savePastedFiles(files, insertPos, docBefore, "拖放");
}

/** 拖放坐标解析为文档插入位置；落在文本块内部时移到块末尾（媒体块是 block 节点，中间插入会拆分文本） */
function resolveDropPos(ed: Editor, clientX: number, clientY: number): number {
  const coords = ed.view.posAtCoords({ left: clientX, top: clientY });
  if (!coords) return ed.state.selection.from;
  const $pos = ed.state.doc.resolve(coords.pos);
  if ($pos.parent.isTextblock && $pos.parentOffset < $pos.parent.content.size) {
    return $pos.end($pos.depth);
  }
  return coords.pos;
}

/** 逐个保存粘贴/拖放的媒体文件为附件（data URL 传输，后端解码落盘），成功后插入媒体块 */
async function savePastedFiles(
  files: { file: File; ext: string }[],
  insertPos: number,
  docBefore: ProseMirrorNode,
  source: "粘贴" | "拖放" = "粘贴",
): Promise<void> {
  if (!props.record) return;
  // 捕获当前记录 id：异步保存期间用户可能切换记录，附件仍落盘到原记录
  const recordId = props.record.id;
  let okCount = 0;
  let failMsg = "";
  let pos = insertPos;
  // 我们自己插入操作后的预期文档：多文件连续插入时文档必然变化（前一张已插入），
  // 与 docBefore 比较会误判「用户编辑」而落到当前选区，导致后插入的图插到前一张前面
  let expectedDoc = docBefore;
  for (const { file, ext } of files) {
    try {
      const dataUrl = await readAsDataURL(file);
      // 保证文件名带正确扩展名（后端按扩展名白名单分类），缺失时补上
      const base = file.name.trim() || `粘贴内容-${Date.now()}`;
      const name = base.toLowerCase().endsWith(`.${ext}`) ? base : `${base}.${ext}`;
      const a = await api.addAttachmentData(recordId, name, dataUrl);
      if (editingId === recordId) {
        // 仍停留在原记录：媒体块插入编辑器；已切换则仅入库（新记录再打开时会加载）
        attachments.value.push(a);
        const ed = editor.value;
        if (ed) {
          // 文档只含我们自己的插入操作时用保存时的位置（多图依次接续），
          // 期间用户另有编辑才落到当前光标处。用 PM 底层 replaceWith：
          // insertContentAt 的 Selection.near 会把光标停在块前
          const target = ed.state.doc.eq(expectedDoc) ? pos : ed.state.selection.from;
          // 块内位置规整到块尾边界：block 节点插入文本块内部会拆分文本块，
          // 拆分后节点实际位置偏移，pos 跟踪会失真。after(depth) 是块结束边界，
          // 而 end(depth) 返回的是块内内容位置（空段落时仍落在块内）
          const $t = ed.state.doc.resolve(target);
          const insertAt = $t.parent.isTextblock ? $t.after($t.depth) : target;
          let tr = ed.state.tr;
          let nodePos = insertAt;
          // 插入点前后紧邻媒体块：atom 相邻时 PM 无合法文本光标位置（TextSelection 要求
          // 父节点含 inlineContent），先补空段落分隔（段落占 2 位，后续节点顺移）
          const $at = ed.state.doc.resolve(insertAt);
          if ($at.nodeBefore?.type.name === "attachment") {
            tr = tr.insert(insertAt, ed.schema.nodes.paragraph.create());
            nodePos = insertAt + 2; // 段落后
          } else if ($at.nodeAfter?.type.name === "attachment") {
            tr = tr.insert(insertAt, ed.schema.nodes.paragraph.create());
            nodePos = insertAt; // 段落前（旧图被顺移到段落之后）
          }
          const attNode = ed.schema.nodes.attachment.create({ id: a.id });
          tr = tr.replaceWith(nodePos, nodePos, attNode);
          ed.view.dispatch(tr);
          // atom（leaf）节点只占 1 个文档位，下一张图接在节点之后
          pos = nodePos + attNode.nodeSize;
          expectedDoc = ed.state.doc;
        }
      }
      okCount++;
    } catch (err) {
      failMsg = errMsg(err);
    }
  }
  if (okCount > 0 && editingId === recordId) placeCaretAfterLastFigure();
  if (okCount > 0) {
    toast({ title: `已添加附件 ${okCount} 个` });
  } else {
    toast({
      title: `${source}失败`,
      description: failMsg || (source === "粘贴" ? "无法读取剪贴板内容" : "无法读取文件"),
      variant: "destructive",
    });
  }
}

/** 光标落到最后插入的媒体块后：块后是文本块则直接聚焦（光标已在块内），
 *  是文档末尾则补一个空段落并把光标移进去（保证媒体块后可继续输入）。
 *  注意：末尾判断必须看 lastChild——docSize 位置的 parent 恒为 doc（非文本块） */
function placeCaretAfterLastFigure(): void {
  const ed = editor.value;
  if (!ed) return;
  const last = ed.state.doc.lastChild;
  if (last && last.isTextblock) {
    ed.view.focus();
    return;
  }
  const docSize = ed.state.doc.content.size;
  const tr = ed.state.tr.insert(docSize, ed.schema.nodes.paragraph.create());
  tr.setSelection(TextSelection.create(tr.doc, docSize + 1));
  ed.view.dispatch(tr);
  ed.view.focus();
}

// ---- 复制媒体块 ----

/**
 * 选中媒体块 Ctrl+C：在默认 HTML 复制之外追加 File 项，
 * 粘贴到访达/聊天工具等外部应用时可直接得到文件。
 * copy 事件是同步的（事件结束后剪贴板即冻结），数据来自选中时的预取缓存，
 * 缓存未命中时仅复制 HTML（粘贴回编辑器仍可还原媒体块）。
 */
function onEditorCopy(view: EditorView, event: Event): boolean {
  const sel = view.state.selection;
  if (!(sel instanceof NodeSelection && sel.node.type.name === "attachment")) {
    return false; // 文字复制走 Tiptap 默认流程
  }
  // 媒体块复制：阻止默认复制，改由 navigator.clipboard.write 写入 HTML + 文件。
  // 原因：WebKit 的 copy 事件不支持 clipboardData.items.add(File)（静默失败不抛异常），
  // 导致「粘贴回编辑器正常、粘贴到外部无文件」；ClipboardItem API 两个平台都支持。
  event.preventDefault();
  const id = sel.node.attrs.id as number | null;
  void copyAttachmentToClipboard(view, sel, id);
  return true;
}

/** 组装 ClipboardItem（HTML + 纯文本 + 文件）写入剪贴板；失败 toast 可见，不静默。
 *  返回是否写入成功（Ctrl+C 不提示成功；右键复制成功时由调用方提示） */
async function copyAttachmentToClipboard(
  view: EditorView,
  sel: NodeSelection,
  id: number | null,
): Promise<boolean> {
  const file = await resolveAttachFile(id);
  if (!file) return false; // 失败原因已在 resolveAttachFile 内 toast
  const { dom, text } = view.serializeForClipboard(sel.content());
  // dom 是 DocumentFragment（无 innerHTML），包一层容器再取 HTML
  const wrap = document.createElement("div");
  wrap.appendChild(dom.cloneNode(true));
  const html = wrap.innerHTML;
  try {
    await navigator.clipboard.write([
      new ClipboardItem({
        "text/html": new Blob([html], { type: "text/html" }),
        "text/plain": new Blob([text || file.name], { type: "text/plain" }),
        [file.type]: file,
      }),
    ]);
    return true;
  } catch (err) {
    toast({
      title: "复制附件失败",
      description: `剪贴板写入被拒绝：${errMsg(err)}`,
      variant: "destructive",
    });
    return false;
  }
}

/** 取附件 File：优先选中时预取的缓存；未命中则实时读取（本地读取 <1s，在用户手势窗口内） */
async function resolveAttachFile(id: number | null): Promise<File | null> {
  if (id == null) {
    toast({ title: "复制附件失败", description: "附件 id 无效", variant: "destructive" });
    return null;
  }
  const cached = attachCopyCache.get(id);
  if (cached) return dataUrlToFile(cached.dataUrl, cached.name);
  try {
    const dataUrl = await api.getAttachmentData(id);
    const name = attachments.value.find((a) => a.id === id)?.file_name ?? "附件";
    return dataUrlToFile(dataUrl, name);
  } catch (err) {
    toast({
      title: "复制附件失败",
      description: `附件数据读取失败：${errMsg(err)}`,
      variant: "destructive",
    });
    return null;
  }
}

// ---- 右键格式菜单 ----

/**
 * 编辑器右键菜单（手动挂在编辑器 DOM 上，见 onCreate，不走 PM 的 handleDOMEvents——
 * 原因：App.vue 全局拦截的 preventDefault 会让 PM 的 eventBelongsToView
 * 判定事件不属于本视图，从而短路 handleDOMEvents 处理器）。
 * - 选中媒体块（attachment 节点）：弹媒体块菜单（删除媒体块）
 * - 选中文字：弹格式菜单（加粗/字号/颜色）
 * - 无选区：不弹菜单
 */
function onEditorContextMenu(_view: EditorView, event: MouseEvent): void {
  const ed = editor.value;
  if (!ed) return;
  event.preventDefault(); // 编辑器区域一律阻止浏览器默认菜单（与 App.vue 全局拦截一致）
  const sel = ed.state.selection;
  const kind =
    sel instanceof NodeSelection && sel.node.type.name === "attachment" ? "media" : "format";
  if (sel.empty || (sel instanceof NodeSelection && kind === "format")) return;
  ctxMenu.value = {
    // 钳制在窗口内，避免菜单溢出屏幕
    x: Math.min(event.clientX, window.innerWidth - 240),
    y: Math.min(event.clientY, window.innerHeight - 160),
    open: true,
    kind,
  };
}

function closeCtxMenu(): void {
  ctxMenu.value = { ...ctxMenu.value, open: false };
}

/** 右键删除媒体块：从文档移除 attachment 节点。
 *  附件物理删除交给孤儿清理机制（onUpdate → 800ms 防抖标记 → 切换记录时删除），
 *  编辑会话内撤销（Cmd/Ctrl+Z）仍能保住附件 */
function deleteSelectedMedia(): void {
  const ed = editor.value;
  if (!ed) return;
  const sel = ed.state.selection;
  if (!(sel instanceof NodeSelection) || sel.node.type.name !== "attachment") return;
  ed.view.dispatch(ed.state.tr.delete(sel.from, sel.to));
  closeCtxMenu();
}

/** 右键复制媒体块：直接写剪贴板（不走 copy 事件），与 Cmd/Ctrl+C 同一条链路 */
function copySelectedMedia(): void {
  const ed = editor.value;
  if (!ed) return;
  const sel = ed.state.selection;
  if (!(sel instanceof NodeSelection) || sel.node.type.name !== "attachment") return;
  const id = sel.node.attrs.id as number | null;
  void copyAttachmentToClipboard(ed.view, sel, id).then((ok) => {
    if (ok) toast({ title: "已复制", description: "可粘贴到聊天窗口 / 文件夹 / 访达" });
  });
  closeCtxMenu();
}

/** 右键导出媒体文件：保存对话框选路径，后端拷贝附件文件；取消对话框不提示 */
async function exportSelectedMedia(): Promise<void> {
  const ed = editor.value;
  if (!ed) return;
  const sel = ed.state.selection;
  if (!(sel instanceof NodeSelection) || sel.node.type.name !== "attachment") return;
  const id = sel.node.attrs.id as number | null;
  closeCtxMenu();
  if (id == null) return;
  try {
    const path = await api.exportAttachment(id);
    if (path) toast({ title: "导出成功", description: `已导出到 ${path}` });
  } catch (e) {
    toast({ title: "导出失败", description: errMsg(e), variant: "destructive" });
  }
}

/** 菜单外点击关闭（面板自身 mousedown 已 preventDefault 保持选区） */
function onWindowMousedown(e: MouseEvent): void {
  if (!ctxMenu.value.open) return;
  if (!(e.target as Element | null)?.closest?.("[data-editor-ctx-menu]")) closeCtxMenu();
}

function applyBold(): void {
  editor.value?.chain().focus().toggleBold().run();
  closeCtxMenu();
}

function applyFontSize(px: string): void {
  editor.value?.chain().focus().setFontSize(px).run();
  closeCtxMenu();
}

function clearFontSize(): void {
  editor.value?.chain().focus().unsetFontSize().run();
  closeCtxMenu();
}

function applyColor(c: string): void {
  editor.value?.chain().focus().setColor(c).run();
  closeCtxMenu();
}

function clearColor(): void {
  editor.value?.chain().focus().unsetColor().run();
  closeCtxMenu();
}

// ---- 孤儿附件清理 ----

/** 从编辑器文档 JSON 收集被引用的附件 id 集合 */
function referencedAttachmentIds(doc: JSONContent): Set<number> {
  const ids = new Set<number>();
  const walk = (n: JSONContent): void => {
    if (n.type === "attachment" && n.attrs && n.attrs.id != null) ids.add(Number(n.attrs.id));
    n.content?.forEach(walk);
  };
  walk(doc);
  return ids;
}

/**
 * 防抖标记孤儿附件：文档不再引用（媒体块被退格删除）的附件进入待删集合。
 * 物理删除延迟到「切换记录」时执行——若用户在编辑会话内 Ctrl+Z 撤销删除，
 * 重新计算引用后附件从待删集合移除，附件得以保住；
 * 切换记录后编辑器内容重置、撤销栈清空，此时删除才是安全的。
 */
function scheduleOrphanCleanup(): void {
  if (orphanCleanupTimer != null) clearTimeout(orphanCleanupTimer);
  orphanCleanupTimer = setTimeout(() => {
    orphanCleanupTimer = null;
    markOrphanAttachments();
  }, 800);
}

/** 计算文档引用差集并更新待删集合（撤销恢复的附件会自动移出） */
function markOrphanAttachments(): void {
  const ed = editor.value;
  if (!ed || props.record == null) return;
  const refs = referencedAttachmentIds(ed.getJSON());
  pendingOrphanIds.clear();
  for (const a of attachments.value) {
    if (!refs.has(a.id)) pendingOrphanIds.add(a.id);
  }
}

/** 物理删除待删集合中的孤儿附件（仅在切换记录、保存成功后调用） */
async function flushPendingOrphans(): Promise<void> {
  const ids = [...pendingOrphanIds];
  pendingOrphanIds.clear();
  for (const id of ids) {
    try {
      await api.deleteAttachment(id);
      attachments.value = attachments.value.filter((x) => x.id !== id);
    } catch (e) {
      toast({ title: "清理附件失败", description: errMsg(e), variant: "destructive" });
    }
  }
}

// ---- 旧数据迁移 ----

/** 相邻媒体块之间补空段落（字符串层处理，载入前执行）：atom 相邻时 PM 无合法文本光标位置
 *  （TextSelection 要求父节点含 inlineContent），方向键会跳过整块、图间点击会选中图片。
 *  在 HTML 层插入而非 setContent 后 dispatch——二次 dispatch 会在 Vue NodeView 渲染 flush 前
 *  触发 PM 的 DOM patch，节点复用错乱导致切换记录时残留上一记录的文本 */
function separateAdjacentAttachments(html: string): string {
  // attachment 是 atom 节点无子内容，</figure> 后紧跟 <figure> 即为相邻媒体块
  return html.replace(/(<figure\b[^>]*>\s*<\/figure>)\s*(?=<figure\b)/g, "$1<p></p>");
}

/** HTML 转义（旧纯文本迁移到富文本时防注入） */
function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/** 编辑器内容初始化：富文本 HTML 直接载入；旧纯文本按行迁移（[附件:N] 行 → figure） */
function htmlForContent(content: string): string {
  if (content.includes("<figure") || content.includes("</p>") || /<p(\s|>)/.test(content)) {
    return content;
  }
  if (content === "") return "";
  return content
    .split("\n")
    .map((line) => {
      const m = line.match(/^\[附件:(\d+)\]$/);
      return m ? `<figure data-attach-id="${m[1]}"></figure>` : `<p>${escapeHtml(line)}</p>`;
    })
    .join("");
}

onMounted(() => {
  window.addEventListener("mousedown", onWindowMousedown, true);
  // useEditor 客户端同步创建；此处兜底一次内容载入（watch immediate 已执行时无副作用）
  editor.value?.commands.setContent(
    separateAdjacentAttachments(htmlForContent(form.content)),
    { emitUpdate: false },
  );
  editor.value?.commands.setTextSelection(0);
});

onBeforeUnmount(() => {
  window.removeEventListener("mousedown", onWindowMousedown, true);
});

// 暴露给父组件：手动保存（Ctrl+S）与切页前兜底
defineExpose({
  flush: () => autosave.flush(),
});
</script>

<template>
  <!-- 已选中记录：编辑表单 -->
  <div v-if="props.record" class="flex h-full flex-col gap-3">
    <!-- 保存状态与操作（flex-wrap：窄窗口时按钮换行而非横向溢出） -->
    <div class="flex flex-wrap items-center justify-between gap-x-2 gap-y-1">
      <div class="flex min-w-0 items-center gap-2 text-xs text-muted-foreground">
        <span v-if="autosave.saving.value" class="flex items-center gap-1">
          <Loader2 class="h-3 w-3 animate-spin" />保存中…
        </span>
        <span v-else-if="autosave.pending.value">有未保存更改</span>
        <span v-else class="truncate">已自动保存 · 更新于 {{ fmtTime(props.record.updated_at) }}</span>
      </div>
      <div class="flex shrink-0 items-center gap-1">
        <!-- 置顶/取消置顶入口（与列表卡片按钮同步状态） -->
        <Button
          variant="ghost"
          size="sm"
          class="gap-1"
          :class="props.record.is_pinned ? 'text-primary' : 'text-muted-foreground'"
          @click="emit('togglePinned')"
        >
          <Pin
            class="h-3.5 w-3.5"
            :class="props.record.is_pinned ? 'fill-primary' : undefined"
          />
          {{ props.record.is_pinned ? "取消置顶" : "置顶" }}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          class="gap-1 text-muted-foreground"
          :disabled="autosave.saving.value"
          @click="manualSave"
        >
          <Save class="h-3.5 w-3.5" />保存 ({{ modLabel }}+S)
        </Button>
        <AlertDialog>
          <AlertDialogTrigger as-child>
            <Button
              variant="ghost"
              size="sm"
              class="gap-1 text-muted-foreground hover:text-destructive"
            >
              <Trash2 class="h-3.5 w-3.5" />删除
            </Button>
          </AlertDialogTrigger>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>移入回收站？</AlertDialogTitle>
              <AlertDialogDescription>
                删除后可在回收站中恢复，不会立即丢失数据。
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>取消</AlertDialogCancel>
              <AlertDialogAction :disabled="deleting" @click="confirmDelete">
                删除
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </div>
    </div>

    <!-- 标题 -->
    <Input
      v-model="form.title"
      placeholder="工作标题（例如：修复登录接口超时）"
      class="border-none bg-transparent px-0 text-lg font-semibold shadow-none focus-visible:ring-0"
    />

    <!-- 标签 -->
    <div
      class="flex flex-wrap items-center gap-1.5 rounded-md border border-dashed px-2 py-1.5 focus-within:border-primary"
    >
      <Badge v-for="t in form.tags" :key="t" variant="secondary" class="gap-1">
        {{ t }}
        <button
          type="button"
          class="rounded-full hover:text-destructive"
          :aria-label="`移除标签 ${t}`"
          @click="removeTag(t)"
        >
          <X class="h-3 w-3" />
        </button>
      </Badge>
      <input
        v-model="tagInput"
        type="text"
        list="tag-suggestions"
        placeholder="输入标签后回车"
        class="min-w-[120px] flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
        @keydown="onTagKeydown"
      />
      <datalist id="tag-suggestions">
        <option v-for="t in store.tags" :key="t.tag" :value="t.tag" />
      </datalist>
    </div>

    <!-- 内容：Tiptap 富文本。粘贴的媒体以媒体块呈现（存储为 figure data-attach-id），
         选中文字右键弹出格式菜单（加粗/字号/颜色） -->
    <div
      class="editor-shell min-h-[240px] flex-1 overflow-y-auto rounded-md border border-input bg-transparent px-3 py-2 text-sm leading-relaxed shadow-none focus-within:ring-1 focus-within:ring-ring"
      @mousedown="onShellMouseDown"
    >
      <EditorContent :editor="editor" />
    </div>

    <div class="text-[11px] text-muted-foreground">
      创建于 {{ fmtTime(props.record.created_at) }} · 停止输入 500ms 自动保存 · 选中文字右键调整格式
    </div>
  </div>

  <!-- 未选中记录：空态 -->
  <div
    v-else
    class="flex h-full flex-col items-center justify-center gap-3 text-center text-muted-foreground"
  >
    <NotebookPen class="h-10 w-10 opacity-30" />
    <p class="text-sm">选择左侧一条记录进行编辑，或新建一条工作记录</p>
    <Button size="sm" class="gap-1" @click="emit('create')">
      <Plus class="h-4 w-4" />新建记录
    </Button>
  </div>

  <!-- 右键菜单（mousedown.prevent 保持文字/媒体选区）：media=删除媒体块；format=加粗/字号/颜色 -->
  <Teleport to="body">
    <div
      v-if="ctxMenu.open"
      data-editor-ctx-menu
      class="fixed z-50 flex flex-col gap-1.5 rounded-md border bg-popover p-2 shadow-md"
      :style="{ left: `${ctxMenu.x}px`, top: `${ctxMenu.y}px` }"
      @mousedown.prevent
    >
      <!-- 媒体块菜单：复制 / 导出 / 删除 -->
      <template v-if="ctxMenu.kind === 'media'">
        <button
          type="button"
          class="flex items-center gap-1.5 rounded px-2 py-1 text-xs hover:bg-accent"
          @click="copySelectedMedia"
        >
          <Copy class="h-3.5 w-3.5" />复制媒体块
        </button>
        <button
          type="button"
          class="flex items-center gap-1.5 rounded px-2 py-1 text-xs hover:bg-accent"
          @click="exportSelectedMedia"
        >
          <Download class="h-3.5 w-3.5" />导出媒体文件
        </button>
        <button
          type="button"
          class="flex items-center gap-1.5 rounded px-2 py-1 text-xs hover:bg-accent hover:text-destructive"
          @click="deleteSelectedMedia"
        >
          <Trash2 class="h-3.5 w-3.5" />删除媒体块
        </button>
      </template>
      <!-- 格式菜单（选中文字） -->
      <template v-else>
        <div class="flex items-center gap-1">
          <button
            type="button"
            class="flex items-center gap-1 rounded px-2 py-1 text-xs hover:bg-accent"
            :class="ctxBold ? 'bg-accent text-accent-foreground' : ''"
            @click="applyBold"
          >
            <Bold class="h-3.5 w-3.5" />加粗
          </button>
        </div>
        <div class="flex items-center gap-1">
          <span class="px-1 text-[10px] text-muted-foreground">字号</span>
          <button
            v-for="s in FONT_SIZES"
            :key="s"
            type="button"
            class="rounded px-1.5 py-0.5 text-xs hover:bg-accent"
            :class="ctxFontSize === s ? 'bg-accent text-accent-foreground' : ''"
            @click="applyFontSize(s)"
          >
            {{ s.replace("px", "") }}
          </button>
          <button
            v-if="ctxFontSize"
            type="button"
            class="rounded px-1.5 py-0.5 text-[10px] text-muted-foreground hover:text-foreground"
            @click="clearFontSize"
          >
            默认
          </button>
        </div>
        <div class="flex items-center gap-1">
          <span class="px-1 text-[10px] text-muted-foreground">颜色</span>
          <button
            v-for="c in COLOR_PALETTE"
            :key="c"
            type="button"
            class="h-5 w-5 rounded-full border border-border"
            :class="ctxColor === c ? 'ring-2 ring-primary' : ''"
            :style="{ backgroundColor: c }"
            :aria-label="`文字颜色 ${c}`"
            @click="applyColor(c)"
          />
          <button
            v-if="ctxColor"
            type="button"
            class="rounded px-1.5 py-0.5 text-[10px] text-muted-foreground hover:text-foreground"
            @click="clearColor"
          >
            清除
          </button>
        </div>
      </template>
    </div>
  </Teleport>
</template>
