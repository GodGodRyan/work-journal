<script setup lang="ts">
/**
 * 富文本工具栏：编辑区顶部格式按钮（撤销重做/标题/粗斜下划删除线/列表/引用/代码/链接/颜色/字号）。
 * 激活态与选区格式：editor 状态本身非响应式，订阅 transaction 事件手动刷新本地 reactive 快照。
 */
import { computed, reactive, ref, watch } from "vue";
import type { Editor } from "@tiptap/core";
import {
  Bold,
  Code,
  Heading1,
  Heading2,
  Heading3,
  Italic,
  Link2,
  List,
  ListOrdered,
  Pilcrow,
  Quote,
  Redo2,
  SquareCode,
  Strikethrough,
  Type,
  Underline,
  Undo2,
} from "@lucide/vue";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent } from "@/components/ui/popover";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { toast } from "@/components/ui/toast";

const props = defineProps<{ editor: Editor | undefined }>();

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

/** 选区格式快照 */
interface FormatState {
  bold: boolean;
  italic: boolean;
  underline: boolean;
  strike: boolean;
  bulletList: boolean;
  orderedList: boolean;
  blockquote: boolean;
  codeBlock: boolean;
  code: boolean;
  link: boolean;
  heading: number | null;
  fontSize: string | null;
  color: string | null;
  canUndo: boolean;
  canRedo: boolean;
}

function collectState(ed: Editor): FormatState {
  return {
    bold: ed.isActive("bold"),
    italic: ed.isActive("italic"),
    underline: ed.isActive("underline"),
    strike: ed.isActive("strike"),
    bulletList: ed.isActive("bulletList"),
    orderedList: ed.isActive("orderedList"),
    blockquote: ed.isActive("blockquote"),
    codeBlock: ed.isActive("codeBlock"),
    code: ed.isActive("code"),
    link: ed.isActive("link"),
    heading: [1, 2, 3].find((l) => ed.isActive("heading", { level: l })) ?? null,
    fontSize: (ed.getAttributes("textStyle").fontSize as string | undefined) ?? null,
    color: (ed.getAttributes("textStyle").color as string | undefined) ?? null,
    canUndo: ed.can().undo(),
    canRedo: ed.can().redo(),
  };
}

const state = reactive<FormatState>({
  bold: false,
  italic: false,
  underline: false,
  strike: false,
  bulletList: false,
  orderedList: false,
  blockquote: false,
  codeBlock: false,
  code: false,
  link: false,
  heading: null,
  fontSize: null,
  color: null,
  canUndo: false,
  canRedo: false,
});

/** 每次事务（输入/格式变更/选区变化）后刷新快照 */
watch(
  () => props.editor,
  (ed, oldEd) => {
    oldEd?.off("transaction", refresh);
    if (!ed) return;
    ed.on("transaction", refresh);
    refresh();
  },
  { immediate: true },
);

function refresh(): void {
  const ed = props.editor;
  if (!ed) return;
  Object.assign(state, collectState(ed));
}

function btnClass(active: boolean): string {
  return active ? "bg-accent text-accent-foreground" : "";
}

/** 标题下拉当前显示 */
const headingLabel = computed(() => {
  const h = state.heading;
  return h != null ? `H${h}` : "正文";
});

/** 字号下拉当前显示 */
const fontSizeLabel = computed(() => {
  const s = state.fontSize;
  return s != null ? s.replace("px", "") : "字号";
});

// ---- 链接 ----

const linkOpen = ref(false);
const linkUrl = ref("");

/** 链接按钮：已有链接则移除；有选区则弹输入框；无选区提示先选文字 */
function onLinkClick(): void {
  const ed = props.editor;
  if (!ed) return;
  if (ed.isActive("link")) {
    ed.chain().focus().unsetLink().run();
    return;
  }
  if (ed.state.selection.empty) {
    toast({ title: "请先选中文字", description: "选中要添加链接的文字后再点击链接按钮" });
    return;
  }
  linkUrl.value = "";
  linkOpen.value = true;
}

/** 应用链接：空输入取消；无 scheme 的地址自动补 https:// */
function applyLink(): void {
  const ed = props.editor;
  const url = linkUrl.value.trim();
  linkOpen.value = false;
  if (!ed || !url) return;
  const href = /^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(url) ? url : `https://${url}`;
  ed.chain().focus().extendMarkRange("link").setLink({ href }).run();
}
</script>

<template>
  <div
    class="flex flex-wrap items-center gap-0.5 border-b border-input px-1.5 py-1"
    @mousedown.prevent
  >
    <!-- 撤销 / 重做 -->
    <Button
      variant="ghost"
      size="icon"
      class="h-7 w-7"
      title="撤销"
      aria-label="撤销"
      :disabled="!state.canUndo"
      @click="props.editor?.chain().focus().undo().run()"
    >
      <Undo2 class="h-4 w-4" />
    </Button>
    <Button
      variant="ghost"
      size="icon"
      class="h-7 w-7"
      title="重做"
      aria-label="重做"
      :disabled="!state.canRedo"
      @click="props.editor?.chain().focus().redo().run()"
    >
      <Redo2 class="h-4 w-4" />
    </Button>

    <span class="mx-0.5 h-4 w-px bg-border" />

    <!-- 标题 -->
    <DropdownMenu>
      <DropdownMenuTrigger as-child>
        <Button
          variant="ghost"
          size="sm"
          class="h-7 gap-1 px-2 text-xs"
          :class="state.heading != null ? 'bg-accent text-accent-foreground' : ''"
        >
          <Heading1 class="h-3.5 w-3.5" />
          {{ headingLabel }}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" class="min-w-[130px]">
        <DropdownMenuLabel class="text-[11px] text-muted-foreground">标题级别</DropdownMenuLabel>
        <DropdownMenuItem
          class="gap-1.5 text-xs"
          :class="state.heading == null ? 'bg-accent' : ''"
          @click="props.editor?.chain().focus().setParagraph().run()"
        >
          <Pilcrow class="h-3.5 w-3.5" />正文
        </DropdownMenuItem>
        <DropdownMenuItem
          v-for="l in [1, 2, 3]"
          :key="l"
          class="gap-1.5 text-xs"
          :class="state.heading === l ? 'bg-accent' : ''"
          @click="props.editor?.chain().focus().toggleHeading({ level: l as 1 | 2 | 3 }).run()"
        >
          <component :is="l === 1 ? Heading1 : l === 2 ? Heading2 : Heading3" class="h-3.5 w-3.5" />
          标题 {{ l }}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>

    <span class="mx-0.5 h-4 w-px bg-border" />

    <!-- 加粗 / 斜体 / 下划线 / 删除线 -->
    <Button
      variant="ghost"
      size="icon"
      class="h-7 w-7"
      :class="btnClass(state.bold)"
      title="加粗 (⌘/Ctrl+B)"
      aria-label="加粗"
      @click="props.editor?.chain().focus().toggleBold().run()"
    >
      <Bold class="h-4 w-4" />
    </Button>
    <Button
      variant="ghost"
      size="icon"
      class="h-7 w-7"
      :class="btnClass(state.italic)"
      title="斜体 (⌘/Ctrl+I)"
      aria-label="斜体"
      @click="props.editor?.chain().focus().toggleItalic().run()"
    >
      <Italic class="h-4 w-4" />
    </Button>
    <Button
      variant="ghost"
      size="icon"
      class="h-7 w-7"
      :class="btnClass(state.underline)"
      title="下划线 (⌘/Ctrl+U)"
      aria-label="下划线"
      @click="props.editor?.chain().focus().toggleUnderline().run()"
    >
      <Underline class="h-4 w-4" />
    </Button>
    <Button
      variant="ghost"
      size="icon"
      class="h-7 w-7"
      :class="btnClass(state.strike)"
      title="删除线"
      aria-label="删除线"
      @click="props.editor?.chain().focus().toggleStrike().run()"
    >
      <Strikethrough class="h-4 w-4" />
    </Button>

    <span class="mx-0.5 h-4 w-px bg-border" />

    <!-- 列表 / 引用 / 代码 -->
    <Button
      variant="ghost"
      size="icon"
      class="h-7 w-7"
      :class="btnClass(state.bulletList)"
      title="无序列表"
      aria-label="无序列表"
      @click="props.editor?.chain().focus().toggleBulletList().run()"
    >
      <List class="h-4 w-4" />
    </Button>
    <Button
      variant="ghost"
      size="icon"
      class="h-7 w-7"
      :class="btnClass(state.orderedList)"
      title="有序列表"
      aria-label="有序列表"
      @click="props.editor?.chain().focus().toggleOrderedList().run()"
    >
      <ListOrdered class="h-4 w-4" />
    </Button>
    <Button
      variant="ghost"
      size="icon"
      class="h-7 w-7"
      :class="btnClass(state.blockquote)"
      title="引用"
      aria-label="引用"
      @click="props.editor?.chain().focus().toggleBlockquote().run()"
    >
      <Quote class="h-4 w-4" />
    </Button>
    <Button
      variant="ghost"
      size="icon"
      class="h-7 w-7"
      :class="btnClass(state.codeBlock)"
      title="代码块"
      aria-label="代码块"
      @click="props.editor?.chain().focus().toggleCodeBlock().run()"
    >
      <SquareCode class="h-4 w-4" />
    </Button>
    <Button
      variant="ghost"
      size="icon"
      class="h-7 w-7"
      :class="btnClass(state.code)"
      title="行内代码"
      aria-label="行内代码"
      @click="props.editor?.chain().focus().toggleCode().run()"
    >
      <Code class="h-4 w-4" />
    </Button>

    <span class="mx-0.5 h-4 w-px bg-border" />

    <!-- 链接：open 完全由 onLinkClick 手动控制（PopoverTrigger 自动开合会与「已有链接则移除」冲突） -->
    <Popover v-model:open="linkOpen">
      <Button
        variant="ghost"
        size="icon"
        class="h-7 w-7"
        :class="btnClass(state.link)"
        title="链接"
        aria-label="链接"
        @click="onLinkClick"
      >
        <Link2 class="h-4 w-4" />
      </Button>
      <PopoverContent align="start" class="flex w-72 items-center gap-1.5 p-2">
        <Input
          v-model="linkUrl"
          placeholder="https://example.com"
          class="h-7 text-xs"
          @keydown.enter="applyLink"
          @keydown.esc="linkOpen = false"
        />
        <Button size="sm" class="h-7 text-xs" @click="applyLink">应用</Button>
      </PopoverContent>
    </Popover>

    <!-- 颜色 -->
    <DropdownMenu>
      <DropdownMenuTrigger as-child>
        <Button
          variant="ghost"
          size="sm"
          class="h-7 gap-1 px-1.5"
          title="文字颜色"
          aria-label="文字颜色"
        >
          <span
            class="inline-block h-3.5 w-3.5 rounded-full border border-border"
            :style="{ backgroundColor: state.color ?? 'hsl(var(--foreground))' }"
          />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" class="min-w-[150px] p-1.5">
        <DropdownMenuLabel class="text-[11px] text-muted-foreground">文字颜色</DropdownMenuLabel>
        <div class="flex flex-wrap gap-1 px-1.5 pb-1.5">
          <button
            v-for="c in COLOR_PALETTE"
            :key="c"
            type="button"
            class="h-5 w-5 rounded-full border border-border"
            :class="state.color === c ? 'ring-2 ring-primary' : ''"
            :style="{ backgroundColor: c }"
            :aria-label="`文字颜色 ${c}`"
            @click="props.editor?.chain().focus().setColor(c).run()"
          />
        </div>
        <DropdownMenuItem
          v-if="state.color"
          class="text-xs text-muted-foreground"
          @click="props.editor?.chain().focus().unsetColor().run()"
        >
          清除颜色
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>

    <!-- 字号 -->
    <DropdownMenu>
      <DropdownMenuTrigger as-child>
        <Button
          variant="ghost"
          size="sm"
          class="h-7 gap-1 px-2 text-xs"
          :class="state.fontSize != null ? 'bg-accent text-accent-foreground' : ''"
          title="字号"
          aria-label="字号"
        >
          <Type class="h-3.5 w-3.5" />
          {{ fontSizeLabel }}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" class="min-w-[110px]">
        <DropdownMenuLabel class="text-[11px] text-muted-foreground">字号</DropdownMenuLabel>
        <DropdownMenuItem
          v-for="s in FONT_SIZES"
          :key="s"
          class="text-xs"
          :class="state.fontSize === s ? 'bg-accent' : ''"
          @click="props.editor?.chain().focus().setFontSize(s).run()"
        >
          {{ s.replace("px", "") }} px
        </DropdownMenuItem>
        <DropdownMenuSeparator v-if="state.fontSize" />
        <DropdownMenuItem
          v-if="state.fontSize"
          class="text-xs text-muted-foreground"
          @click="props.editor?.chain().focus().unsetFontSize().run()"
        >
          默认字号
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  </div>
</template>
