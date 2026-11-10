<script setup lang="ts">
/**
 * 记录编辑器：标题 / 标签 / 富文本内容（RichTextEditor 组件）。
 * - 停止输入 500ms 自动保存（useAutoSave），保存按钮与 ⌘/Ctrl+S 手动兜底（跟随系统）
 * - 切换记录先 flush 未落盘内容（通过 captured editingId 保存到正确的记录），再清理孤儿附件
 * - 删除走软删除（AlertDialog 确认），数据可到回收站恢复
 * - 置顶 / 移动到其他日期（日历弹窗选目标日期）
 */
import { computed, nextTick, reactive, ref, watch } from "vue";
import { CalendarDays, Loader2, NotebookPen, Pin, Plus, Save, Trash2, X } from "@lucide/vue";
import { parseDate } from "@internationalized/date";
import type { DateValue } from "reka-ui";
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
import { Calendar } from "@/components/ui/calendar";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { toast } from "@/components/ui/toast";
import * as api from "@/api/records";
import { useAutoSave } from "@/composables/useAutoSave";
import { modLabel } from "@/composables/useHotkeys";
import { useAppStore } from "@/stores/app";
import { errMsg, joinTags, splitTags } from "@/lib/utils";
import { dateValueToStr, fmtTime, todayStr } from "@/composables/useDate";
import type { WorkRecord } from "@/types/work-record";
import RichTextEditor from "@/components/editor/RichTextEditor.vue";

const props = defineProps<{ record: WorkRecord | null }>();

const emit = defineEmits<{
  saved: [r: WorkRecord];
  deleted: [];
  create: [];
  moved: [r: WorkRecord];
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

/** 富文本编辑器实例（切换记录时清理孤儿附件用） */
const richEditor = ref<InstanceType<typeof RichTextEditor> | null>(null);

/** 当前正在编辑的记录 id（保存时使用，切换记录后仍能落盘到正确记录） */
let editingId: number | null = null;
/** 从 props 同步表单时置位，抑制变更监听 */
let syncing = false;

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

// 切换记录：先保存上一条（成功后才清理孤儿附件，失败宁可残留不误删），再同步新记录表单
watch(
  () => props.record?.id,
  async (id, oldId) => {
    if (oldId != null && oldId !== id) {
      // await 前绑定 flushOrphans：autosave.flush 的 await 让出微任务后，
      // Vue 可能已完成重渲染（v-if 卸载 RichTextEditor → ref 置 null），
      // 若 await 后再取 richEditor.value 会拿到 null 而跳过孤儿清理
      const flushOrphans = richEditor.value?.flushOrphans.bind(richEditor.value);
      const ok = await autosave.flush();
      // 此时编辑器仍是旧记录内容：撤销栈即将清空，物理删除孤儿附件安全
      if (ok && flushOrphans) await flushOrphans();
    }
    // await 让出期间可能又切换了记录（如新建流程 1→undefined→2 两次触发交错执行），
    // 过期回调的表单赋值会覆盖新回调的 editingId/form，必须放弃
    if (props.record?.id !== id) return;
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

// ---- 移动到其他日期 ----

const moveOpen = ref(false);
const moving = ref(false);
/** 日历选中值：每次打开弹窗时重置为当前记录日期 */
const moveDate = ref(parseDate(todayStr()));

watch(moveOpen, (v) => {
  if (v) moveDate.value = parseDate(props.record?.record_date ?? todayStr());
});

/** 传给 Calendar 的 model-value：运行时与 moveDate 同值，仅 as 断言满足 reka-ui 的名义类型检查 */
const calendarModel = computed<DateValue>(() => moveDate.value as unknown as DateValue);

/** 日历点选目标日期：先落盘未保存内容，再移动记录并通知主页跳转 */
async function onMoveDate(d: DateValue | undefined): Promise<void> {
  moveOpen.value = false;
  if (!d || !props.record || moving.value) return;
  const target = dateValueToStr(d);
  if (target === props.record.record_date) return; // 选回原日期：无操作
  moving.value = true;
  try {
    // 先落盘未保存的标题/内容（flush 失败已由 useAutoSave 的 onError toast 提示，不再重复）
    const ok = await autosave.flush();
    if (!ok) return;
    const updated = await api.setRecordDate(props.record.id, target);
    emit("moved", updated);
    toast({ title: "已移动", description: `已移动到 ${target}` });
  } catch (e) {
    toast({ title: "移动失败", description: errMsg(e), variant: "destructive" });
  } finally {
    moving.value = false;
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
        <!-- 移动到其他日期：日历弹窗选目标日期 -->
        <Popover v-model:open="moveOpen">
          <PopoverTrigger as-child>
            <Button
              variant="ghost"
              size="sm"
              class="gap-1 text-muted-foreground"
              :disabled="moving"
            >
              <Loader2 v-if="moving" class="h-3.5 w-3.5 animate-spin" />
              <CalendarDays v-else class="h-3.5 w-3.5" />
              移动到…
            </Button>
          </PopoverTrigger>
          <PopoverContent align="end" class="w-auto p-0">
            <!-- 单向绑定：选中后弹窗立即关闭，无需把 DateValue 写回 CalendarDate 类型的 ref -->
            <Calendar :model-value="calendarModel" @update:model-value="onMoveDate" />
          </PopoverContent>
        </Popover>
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

    <!-- 内容：富文本编辑器（工具栏 + 编辑区 + 媒体块） -->
    <div class="min-h-0 flex-1 overflow-hidden rounded-md border border-input shadow-none">
      <RichTextEditor
        ref="richEditor"
        :record-id="props.record.id"
        :content="form.content"
        @update:content="form.content = $event"
      />
    </div>

    <div class="text-[11px] text-muted-foreground">
      创建于 {{ fmtTime(props.record.created_at) }} · 停止输入 500ms 自动保存 · 工具栏调整格式
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
</template>
