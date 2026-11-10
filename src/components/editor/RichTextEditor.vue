<script setup lang="ts">
/**
 * 富文本编辑器（Tiptap）：工具栏 + 编辑区 + 媒体块，通过 v-model 与父组件同步 HTML。
 * - 全量格式：标题/加粗/斜体/下划线/删除线/列表/引用/代码/链接/颜色/字号
 * - 内容以 HTML 存储（<figure data-attach-id> 为媒体块），与旧数据完全兼容
 * - 粘贴/拖放媒体文件：异步保存为附件后插入媒体块；普通粘贴走 Tiptap 默认（保留来源基本格式）
 * - 媒体块删除后附件延迟物理删除：切换记录（撤销栈清空）时由父组件调 flushOrphans
 * - App.vue 在 window 捕获阶段 preventDefault 了 contextmenu/dragover/drop（桌面应用体验），
 *   导致 ProseMirror 内置处理器被短路，因此文件拖放需在编辑器 DOM 上手动挂监听
 */
import { onBeforeUnmount, onMounted, provide, ref, watch } from "vue";
import { EditorContent, useEditor } from "@tiptap/vue-3";
import StarterKit from "@tiptap/starter-kit";
import { Color, FontSize, TextStyle } from "@tiptap/extension-text-style";
import { NodeSelection, TextSelection } from "@tiptap/pm/state";
import type { Plugin } from "@tiptap/pm/state";
import { history } from "@tiptap/pm/history";
import type { EditorView } from "@tiptap/pm/view";
import type { Node as ProseMirrorNode } from "@tiptap/pm/model";
import type { Editor } from "@tiptap/vue-3";
import { toast } from "@/components/ui/toast";
import * as api from "@/api/records";
import { errMsg } from "@/lib/utils";
import type { Attachment } from "@/types/work-record";
import AttachmentExtension from "@/components/editor/Attachment";
import EditorToolbar from "@/components/editor/EditorToolbar.vue";
import { attachCopyCache, dataUrlToFile } from "@/components/editor/attach-copy-cache";

const props = defineProps<{
  /** 当前记录 id：粘贴/拖放媒体入库用；null 时（未选记录）不接管媒体粘贴 */
  recordId: number | null;
  /** 富文本 HTML（与父组件 form.content 双向绑定） */
  content: string;
}>();

const emit = defineEmits<{ "update:content": [html: string] }>();

/** 当前记录附件列表（MediaBlock NodeView 通过 inject 读取元数据） */
const attachments = ref<Attachment[]>([]);
provide("editor-attachments", attachments);

/** MIME → 扩展名（与后端白名单对齐；仅白名单内类型可粘贴为附件） */
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

/** 由文件名扩展名推断类型（文件粘贴时 MIME 可能为空） */
function extFromName(name: string): string | null {
  const m = name.toLowerCase().match(/\.([a-z0-9]+)$/);
  if (!m) return null;
  return Object.values(MIME_EXT).includes(m[1]) ? m[1] : null;
}

function readAsDataURL(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const fr = new FileReader();
    fr.onload = () => resolve(fr.result as string);
    fr.onerror = () => reject(new Error("读取文件内容失败"));
    fr.readAsDataURL(file);
  });
}

// ---- Tiptap 编辑器 ----

const editor = useEditor({
  extensions: [
    // StarterKit 全量格式；trailingNode 默认启用：文档末尾恒有空段落，媒体块在末尾时也可继续输入
    StarterKit.configure({
      heading: { levels: [1, 2, 3] },
      link: { openOnClick: false, autolink: true },
    }),
    TextStyle,
    FontSize,
    Color,
    AttachmentExtension,
  ],
  content: "",
  editorProps: {
    attributes: {
      class: "tiptap focus:outline-none",
      "data-gramm": "false",
      "data-placeholder": "工作内容（工具栏可调整格式，支持粘贴/拖入图片与媒体文件）…",
    },
    handlePaste: onPaste,
    handleDOMEvents: {
      copy: onEditorCopy,
    },
  },
  onCreate: ({ editor: ed }) => {
    // 文件拖放：App.vue 全局捕获阶段 preventDefault 使 PM 内置 drop 处理器短路，
    // 因此手动挂监听自己处理（dragover 必须 preventDefault 才会触发 drop）
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
      onEditorDrop(event);
    });
  },
  onUpdate: ({ editor: ed }) => {
    emit("update:content", ed.getHTML());
    scheduleCollectOrphans();
  },
});

/** 点击编辑区空白（shell padding）聚焦到文档末尾 */
function onShellMouseDown(e: MouseEvent): void {
  const ed = editor.value;
  if (!ed) return;
  if (ed.view.dom.contains(e.target as Node)) return; // PM 内部区域由自身处理光标
  e.preventDefault();
  ed.commands.focus("end");
}

// ---- 内容载入（含旧数据迁移） ----

/** HTML 转义（旧纯文本迁移到富文本时防注入） */
function escapeHtml(s: string): string {
  return s
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/** 相邻媒体块之间补空段落：atom 相邻时 PM 无合法文本光标位置（方向键跳块、图间点击误选）。
 *  载入时对旧数据做一次性修补；新数据由 insertAttachmentNode 在插入时保证不产生相邻媒体块 */
function normalizeAdjacentAttachments(html: string): string {
  return html.replace(/(<figure\b[^>]*>\s*<\/figure>)\s*(?=<figure\b)/g, "$1<p></p>");
}

/** 编辑器内容初始化：富文本 HTML 直接载入；旧纯文本按行迁移（[附件:N] 行 → figure） */
function contentToHtml(content: string): string {
  if (content.includes("<figure") || content.includes("</p>") || /<p(\s|>)/.test(content)) {
    return normalizeAdjacentAttachments(content);
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

/** 文档中第一个文本块内的位置（setContent 后选区用；文档开头是媒体块时 TextSelection.create 会抛错） */
function firstTextPos(doc: ProseMirrorNode): number {
  let target = 0;
  doc.descendants((node, pos) => {
    if (node.isTextblock) {
      target = pos + 1;
      return false;
    }
    return true;
  });
  return target;
}

/** 是否为 undo/redo 历史插件（prosemirror-history 的 key 字符串为 "history$"，类型未暴露，断言读取） */
function isHistoryPlugin(p: Plugin): boolean {
  return (((p.spec.key as { key?: string } | undefined)?.key ?? "").startsWith("history$"));
}

/**
 * 清空撤销/重做栈：注册全新 history 插件替换旧 state（done/undone 归零）。
 * registerPlugin 的 handlePlugins 自定义插件数组，一次 reconfigure 完成替换。
 * 必须在载入内容后调用：载入不记历史，且旧记录的编辑历史不得逆作用到新文档
 */
function clearUndoHistory(ed: Editor): void {
  ed.registerPlugin(history({ depth: 100, newGroupDelay: 500 }), (plugin, plugins) => [
    ...plugins.filter((p) => !isHistoryPlugin(p)),
    plugin,
  ]);
}

/**
 * 载入 content prop：setContent 的选区映射会把开头是媒体块时的选区变成 NodeSelection（误选中），
 * 显式把选区放到第一个文本块内。
 * 注意跳过「自己 emit 的回显」：编辑器 onUpdate emit 的 HTML 会经 v-model 回流到这里，
 * 与 getHTML() 相等时必须直接返回，否则每次输入都会 setContent 重置光标到文档开头
 */
function loadContent(): void {
  const ed = editor.value;
  if (!ed) return;
  if (ed.getHTML() === props.content) return;
  ed.chain()
    .setMeta("addToHistory", false)
    .setContent(contentToHtml(props.content), { emitUpdate: false })
    .run();
  ed.commands.setTextSelection(firstTextPos(ed.state.doc));
  clearUndoHistory(ed);
}

watch(() => props.content, loadContent, { immediate: true });

// ---- 附件列表 ----

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

watch(() => props.recordId, (id) => void loadAttachments(id), { immediate: true });

// ---- 粘贴 / 拖放媒体 ----

interface MediaFile {
  file: File;
  ext: string;
}

/** 白名单过滤文件列表（不支持的类型逐个 toast 提示） */
function parseFiles(raw: File[]): MediaFile[] {
  const files: MediaFile[] = [];
  for (const f of raw) {
    const ext = MIME_EXT[f.type] ?? extFromName(f.name);
    if (!ext) {
      toast({
        title: "无法识别的媒体文件",
        description: f.name || f.type || "未知文件",
        variant: "destructive",
      });
      continue;
    }
    files.push({ file: f, ext });
  }
  return files;
}

/**
 * 粘贴统一接管策略：
 * - HTML 含 data-attach-id（编辑器内部复制的媒体块）：默认粘贴还原节点，剪贴板 File 项忽略防重复入库
 * - 无媒体文件：默认粘贴（保留来源基本格式，PM schema 过滤未知标签）
 * - 有媒体文件：接管。文本经 PM 标准 pasteText 插入（换行按 schema 拆段），媒体异步入库后插入
 */
function onPaste(view: EditorView, event: ClipboardEvent): boolean {
  if (props.recordId == null) return false;
  const html = event.clipboardData?.getData("text/html") ?? "";
  if (html.includes("data-attach-id")) return false;

  const items = Array.from(event.clipboardData?.items ?? []);
  const rawFiles = items.filter((i) => i.kind === "file");
  if (rawFiles.length === 0) return false;

  event.preventDefault();
  const text = event.clipboardData?.getData("text/plain") ?? "";
  if (text) view.pasteText(text);
  const files = parseFiles(
    rawFiles.map((i) => i.getAsFile()).filter((f): f is File => f != null),
  );
  if (files.length > 0) void saveMediaFiles(files, view.state.selection.from, "粘贴");
  return true;
}

/** 拖放：坐标解析为插入位置；落在文本块中间时规整到块末尾（block 节点中间插入会拆分文本块） */
function onEditorDrop(event: DragEvent): void {
  const ed = editor.value;
  if (!ed || props.recordId == null) return;
  event.preventDefault();
  ed.view.focus();
  const files = parseFiles(Array.from(event.dataTransfer?.files ?? []));
  if (files.length === 0) return;
  const coords = ed.view.posAtCoords({ left: event.clientX, top: event.clientY });
  let pos = coords ? coords.pos : ed.state.selection.from;
  const $pos = ed.state.doc.resolve(pos);
  if ($pos.parent.isTextblock && $pos.parentOffset < $pos.parent.content.size) {
    pos = $pos.end($pos.depth);
  }
  void saveMediaFiles(files, pos, "拖放");
}

/**
 * 在指定位置插入媒体块节点，返回新节点之后的文档位置（连续插入的接续锚点）。
 * PM 约束：位置落在文本块内部时规整到块结束边界；插入点前后紧邻媒体块时先补空段落分隔
 */
function insertAttachmentNode(id: number, pos: number): number {
  const ed = editor.value;
  if (!ed) return pos;
  const doc = ed.state.doc;
  const target = Math.min(Math.max(pos, 0), doc.content.size);
  const $target = doc.resolve(target);
  const insertAt = $target.parent.isTextblock ? $target.after($target.depth) : target;

  let tr = ed.state.tr;
  let nodePos = insertAt;
  const $at = doc.resolve(insertAt);
  if ($at.nodeBefore?.type.name === "attachment") {
    tr = tr.insert(insertAt, ed.schema.nodes.paragraph.create());
    nodePos = insertAt + 2;
  } else if ($at.nodeAfter?.type.name === "attachment") {
    tr = tr.insert(insertAt, ed.schema.nodes.paragraph.create());
    nodePos = insertAt;
  }
  const node = ed.schema.nodes.attachment.create({ id });
  tr = tr.replaceWith(nodePos, nodePos, node);
  const afterPos = nodePos + node.nodeSize;
  // 节点插入后位于文档末尾（后面没有文本块）：tr 中补空段落供光标落点。
  // 不能在 dispatch 后靠 trailingNode：setSelection 在 dispatch 前执行，tr.doc 上还没有末尾段落，
  // TextSelection.create 对非文本块位置会抛错
  let caret = afterPos;
  if (tr.doc.resolve(afterPos).nodeAfter == null) {
    tr = tr.insert(afterPos, ed.schema.nodes.paragraph.create());
    caret = afterPos; // 段落起始位置（resolve 后 parent 为段落，合法文本位置）
  }
  tr.setSelection(TextSelection.create(tr.doc, caret));
  ed.view.dispatch(tr);
  return afterPos;
}

/** 逐个保存媒体文件为附件（data URL 传输，后端解码落盘），成功后插入媒体块 */
async function saveMediaFiles(
  files: MediaFile[],
  insertPos: number,
  source: "粘贴" | "拖放",
): Promise<void> {
  const recordId = props.recordId;
  if (recordId == null) return;
  let okCount = 0;
  let failMsg = "";
  let pos = insertPos;
  for (const { file, ext } of files) {
    try {
      const dataUrl = await readAsDataURL(file);
      // 保证文件名带正确扩展名（后端按扩展名白名单分类），缺失时补上
      const base = file.name.trim() || `${source}内容-${Date.now()}`;
      const name = base.toLowerCase().endsWith(`.${ext}`) ? base : `${base}.${ext}`;
      const a = await api.addAttachmentData(recordId, name, dataUrl);
      okCount++;
      // 异步期间用户已切换记录：附件已入库到原记录，不再插入编辑器
      if (props.recordId !== recordId) continue;
      attachments.value.push(a);
      if (editor.value) pos = insertAttachmentNode(a.id, pos);
    } catch (err) {
      failMsg = errMsg(err);
    }
  }
  editor.value?.view.focus();
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

// ---- 复制媒体块 ----

/**
 * 选中媒体块 Ctrl+C：在默认 HTML 复制之外追加 File 项，粘贴到外部应用可直接得到文件。
 * copy 事件是同步的（事件结束后剪贴板冻结），数据来自选中时预取的缓存，未命中时仅复制 HTML。
 * WebKit 的 copy 事件不支持 clipboardData.items.add(File)，必须用 ClipboardItem API。
 */
function onEditorCopy(view: EditorView, event: Event): boolean {
  const sel = view.state.selection;
  if (!(sel instanceof NodeSelection && sel.node.type.name === "attachment")) return false;
  event.preventDefault();
  const id = sel.node.attrs.id as number | null;
  void copyAttachmentToClipboard(view, sel, id);
  return true;
}

/** 组装 ClipboardItem（HTML + 纯文本 + 文件）写入剪贴板；失败 toast 可见，不静默 */
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

/** 文档中指定附件 id 的节点位置 */
function findAttachmentPos(id: number): number | null {
  const ed = editor.value;
  if (!ed) return null;
  let found: number | null = null;
  ed.state.doc.descendants((node, pos) => {
    if (found != null) return false;
    if (node.type.name === "attachment" && node.attrs.id === id) {
      found = pos;
      return false;
    }
    return true;
  });
  return found;
}

/** MediaBlock hover 操作条「复制」：构造节点选区走与 Ctrl+C 相同的剪贴板写入链路 */
async function copyAttachmentById(id: number | null): Promise<void> {
  const ed = editor.value;
  if (!ed || id == null) return;
  const pos = findAttachmentPos(id);
  if (pos == null) return;
  const sel = NodeSelection.create(ed.state.doc, pos);
  const ok = await copyAttachmentToClipboard(ed.view, sel, id);
  if (ok) toast({ title: "已复制", description: "可粘贴到聊天窗口 / 文件夹 / 访达" });
}
provide("attachment-copy", copyAttachmentById);

// ---- 孤儿附件清理 ----

/** 从编辑器文档 JSON 收集被引用的附件 id 集合 */
function referencedAttachmentIds(doc: { content?: unknown[] }): Set<number> {
  const ids = new Set<number>();
  const walk = (n: Record<string, unknown>): void => {
    if (n.type === "attachment" && n.attrs != null) {
      const id = (n.attrs as Record<string, unknown>).id;
      if (id != null) ids.add(Number(id));
    }
    const children = n.content;
    if (Array.isArray(children)) children.forEach((c) => walk(c as Record<string, unknown>));
  };
  (doc.content ?? []).forEach((c) => walk(c as Record<string, unknown>));
  return ids;
}

/** 孤儿附件待删集合：媒体块从文档移除后延迟物理删除，编辑会话内撤销可恢复 */
const orphanIds = new Set<number>();
let orphanTimer: ReturnType<typeof setTimeout> | null = null;

/** 防抖收集：文档不再引用的附件进入待删集合（撤销恢复的附件自动移出） */
function scheduleCollectOrphans(): void {
  if (orphanTimer != null) clearTimeout(orphanTimer);
  orphanTimer = setTimeout(() => {
    orphanTimer = null;
    const ed = editor.value;
    if (!ed) return;
    const refs = referencedAttachmentIds(ed.getJSON());
    orphanIds.clear();
    for (const a of attachments.value) {
      if (!refs.has(a.id)) orphanIds.add(a.id);
    }
  }, 500);
}

/**
 * 物理删除待删集合中的孤儿附件。仅在「切换记录」时由父组件调用：
 * 切换记录会清空撤销栈，此时删除不会破坏 Ctrl+Z 恢复媒体块
 */
async function flushOrphans(): Promise<void> {
  const ids = [...orphanIds];
  orphanIds.clear();
  for (const id of ids) {
    try {
      await api.deleteAttachment(id);
      attachments.value = attachments.value.filter((x) => x.id !== id);
    } catch (e) {
      toast({ title: "清理附件失败", description: errMsg(e), variant: "destructive" });
    }
  }
}

defineExpose({ flushOrphans });

// useEditor 在 onMounted 才创建编辑器（setup 时 editor 为 null，watch immediate 会静默跳过）；
// 此处兜底一次内容载入
onMounted(loadContent);

onBeforeUnmount(() => {
  if (orphanTimer != null) clearTimeout(orphanTimer);
});
</script>

<template>
  <div class="flex h-full min-h-0 flex-col">
    <EditorToolbar :editor="editor" />
    <div
      class="editor-shell min-h-[220px] flex-1 overflow-y-auto rounded-b-md px-3 py-2 text-sm leading-relaxed focus-within:ring-1 focus-within:ring-ring"
      @mousedown="onShellMouseDown"
    >
      <EditorContent :editor="editor" />
    </div>
  </div>
</template>
