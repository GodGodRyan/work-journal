<script setup lang="ts">
/**
 * 主页（日期编辑视图）：
 * 左：侧边栏（置顶区 / 日历 / 标签）
 * 右：日期导航（单日或范围）+ 记录列表 + 新建/编辑表单（500ms 防抖自动保存）
 * 支持从其他页面携带 ?date=yyyy-MM-dd 跳入（时间线/搜索跳转编辑）。
 * 范围视图（store.dateRange 非空）：列表按日期分组（置顶组在最前），
 * 不支持新建与导出（口径锚定单日），退出范围回到单日视图。
 */
import { computed, nextTick, onMounted, reactive, ref, watch } from "vue";
import { useRoute } from "vue-router";
import {
  CalendarDays,
  CalendarRange,
  ChevronLeft,
  ChevronRight,
  Download,
  Plus,
} from "@lucide/vue";
import { Button } from "@/components/ui/button";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Skeleton } from "@/components/ui/skeleton";
import { toast } from "@/components/ui/toast";
import PinnedList from "@/components/PinnedList.vue";
import RecordCalendar from "@/components/RecordCalendar.vue";
import RecordCard from "@/components/RecordCard.vue";
import RecordEditor from "@/components/RecordEditor.vue";
import TagList from "@/components/TagList.vue";
import TagRecordsDialog from "@/components/TagRecordsDialog.vue";
import * as api from "@/api/records";
import { useAppStore } from "@/stores/app";
import { addDaysStr, fmtDateCn, isToday, todayStr } from "@/composables/useDate";
import { modLabel } from "@/composables/useHotkeys";
import { errMsg } from "@/lib/utils";
import type { ExportKind, WorkRecord } from "@/types/work-record";

const store = useAppStore();
const route = useRoute();
const selectedId = ref<number | null>(null);
const editorRef = ref<InstanceType<typeof RecordEditor> | null>(null);

const selectedRecord = computed(
  () => store.filteredRecords.find((r) => r.id === selectedId.value) ?? null,
);

/** 范围视图的分组：仅按日期分组（置顶记录归入各自日期组，组内保持后端顺序=置顶在前） */
const rangeGroups = computed(() => {
  const list = store.filteredRecords;
  const groups: { key: string; title: string; items: WorkRecord[] }[] = [];
  for (const r of list) {
    const last = groups[groups.length - 1];
    if (last && last.title === r.record_date) last.items.push(r);
    else groups.push({ key: r.record_date, title: r.record_date, items: [r] });
  }
  return groups;
});

async function loadData(): Promise<void> {
  try {
    await store.loadRecords();
    const list = store.filteredRecords;
    if (list.length > 0 && !list.some((r) => r.id === selectedId.value)) {
      selectedId.value = list[0].id;
    } else if (list.length === 0) {
      selectedId.value = null;
    }
  } catch (e) {
    toast({ title: "加载记录失败", description: errMsg(e), variant: "destructive" });
  }
}

async function loadSidebar(): Promise<void> {
  try {
    await store.loadSidebar();
  } catch (e) {
    toast({ title: "加载侧边栏失败", description: errMsg(e), variant: "destructive" });
  }
}

onMounted(() => {
  // 支持 ?date=yyyy-MM-dd 跳转（时间线/搜索结果跳入）
  const qd = route.query.date;
  if (typeof qd === "string" && /^\d{4}-\d{2}-\d{2}$/.test(qd) && qd !== store.currentDate) {
    store.setDate(qd); // watch(currentDate) 会触发加载
  } else {
    void loadData();
  }
  void loadSidebar();
});

// 切换日期或进出范围：清空选中并重新加载记录。
// 组合键保证「退出范围时 currentDate 不变」也能触发（dateRange 由非空变 null）。
watch(
  () =>
    store.dateRange ? `${store.dateRange.from}~${store.dateRange.to}` : store.currentDate,
  () => {
    selectedId.value = null;
    void loadData();
  },
);

// 标签筛选变化：重载列表
watch(
  () => store.tagFilter,
  () => {
    void loadData();
  },
);

/** 新建记录：立即入库空记录并选中（后续编辑走自动保存） */
async function createRecord(): Promise<void> {
  if (store.dateRange) {
    toast({ title: "范围视图不支持新建", description: "请先退出范围视图，回到单日再新建记录" });
    return;
  }
  try {
    const r = await api.createRecord(store.currentDate, "", "", "pending", "");
    selectedId.value = r.id;
    await store.refreshAll();
  } catch (e) {
    toast({ title: "新建失败", description: errMsg(e), variant: "destructive" });
  }
}

/** 置顶/取消置顶（置顶后记录按后端排序自动移到列表最前，选中状态不受影响） */
async function togglePinned(id: number): Promise<void> {
  const r = store.records.find((x) => x.id === id);
  if (!r) return;
  try {
    const updated = await api.setPinned(id, !r.is_pinned);
    const i = store.records.findIndex((x) => x.id === id);
    if (i >= 0) store.records[i] = updated;
    void store.loadPinned().catch(() => undefined); // 同步侧边栏置顶区
  } catch (e) {
    toast({ title: "置顶操作失败", description: errMsg(e), variant: "destructive" });
  }
}

/**
 * 点击侧边栏置顶区记录：跳转到所属日期并选中编辑。
 * setDate 触发的 watch 回调会把 selectedId 清空并异步加载该日记录，
 * 因此先 await nextTick（等 watch 回调执行完），再设置目标 id；
 * loadData 完成时发现 selectedId 在列表中会保留它。
 */
async function jumpToPinned(r: WorkRecord): Promise<void> {
  store.setDate(r.record_date);
  await nextTick();
  selectedId.value = r.id;
}

// ---- 标签弹窗视图 ----
const tagDialog = reactive({ open: false, tag: "" });

function openTagDialog(tag: string): void {
  tagDialog.tag = tag;
  tagDialog.open = true;
}

/** 标签弹窗内点击记录：关闭弹窗并复用跳转选中模式（setDate → nextTick → selectedId） */
function onTagJump(r: WorkRecord): void {
  tagDialog.open = false;
  void jumpToPinned(r);
}

/** 自动/手动保存成功：更新列表并刷新标签 */
function onSaved(r: WorkRecord): void {
  const i = store.records.findIndex((x) => x.id === r.id);
  if (i >= 0) store.records[i] = r;
  // 侧边栏置顶区是独立查询的列表（listPinned），同样就地替换，
  // 否则置顶记录改标题后置顶区仍显示旧标题
  const p = store.pinnedRecords.findIndex((x) => x.id === r.id);
  if (p >= 0) {
    // 保存不会改变置顶状态；若记录已非置顶（其他入口操作后置顶区尚未刷新），就地移除
    if (r.is_pinned) store.pinnedRecords[p] = r;
    else store.pinnedRecords.splice(p, 1);
  }
  void store.loadTags().catch(() => undefined);
}

async function onDeleted(): Promise<void> {
  await store.refreshAll();
  selectedId.value = store.filteredRecords[0]?.id ?? null;
}

// ---- 列表右键删除（确认后软删除，与编辑器删除一致） ----
// 弹窗开关与删除目标分离：点「删除」时 reka 会先触发 update:open(false)，
// 若在该回调里清 deleteTarget，confirmListDelete 将读取到 null 而静默跳过
const deleteDialogOpen = ref(false);
const deleteTarget = ref<WorkRecord | null>(null);
const deletingFromList = ref(false);

async function confirmListDelete(): Promise<void> {
  const r = deleteTarget.value;
  if (!r) return;
  deletingFromList.value = true;
  try {
    await api.softDelete(r.id);
    await store.refreshAll();
    selectedId.value = store.filteredRecords[0]?.id ?? null;
    toast({ title: "已移入回收站", description: "可在回收站页面恢复" });
  } catch (e) {
    toast({ title: "删除失败", description: errMsg(e), variant: "destructive" });
  } finally {
    deletingFromList.value = false;
    deleteDialogOpen.value = false;
    deleteTarget.value = null;
  }
}

// ---- 全局快捷键信号（来自 App.vue） ----

watch(
  () => store.newRecordSignal,
  () => {
    void createRecord();
  },
);

watch(
  () => store.manualSaveSignal,
  () => {
    void editorRef.value?.flush();
  },
);

// ---- 导出 ----

async function exportReport(kind: ExportKind): Promise<void> {
  try {
    const path = await api.exportReport(kind, store.currentDate);
    if (path) toast({ title: "导出成功", description: path });
  } catch (e) {
    toast({ title: "导出失败", description: errMsg(e), variant: "destructive" });
  }
}
</script>

<template>
  <div class="flex h-full">
    <!-- 左侧边栏：日历 / 标签 -->
    <aside class="flex w-[248px] shrink-0 flex-col gap-4 overflow-y-auto border-r bg-card p-3">
      <!-- 置顶区：固定显示全部置顶记录（点击跳转选中，右键删除走下方确认对话框） -->
      <PinnedList
        @select="jumpToPinned"
        @delete="(r) => { deleteTarget = r; deleteDialogOpen = true; }"
      />
      <div class="rounded-md border">
        <RecordCalendar />
      </div>
      <TagList @select-tag="openTagDialog" />
    </aside>

    <!-- 右侧主区域 -->
    <section class="flex min-w-0 flex-1 flex-col">
      <!-- 日期导航 + 导出 + 新建 -->
      <header class="flex h-14 shrink-0 items-center justify-between gap-2 border-b px-4">
        <!-- 范围视图：显示范围 + 退出按钮 -->
        <div v-if="store.dateRange" class="flex items-center gap-1">
          <span class="ml-1 flex items-center gap-2 text-sm font-medium">
            <CalendarRange class="h-4 w-4 text-muted-foreground" />
            {{ store.dateRange.from }} ~ {{ store.dateRange.to }}
            <Button
              variant="outline"
              size="sm"
              class="h-6 px-2 text-xs"
              @click="store.exitRange()"
            >
              退出范围
            </Button>
          </span>
        </div>
        <!-- 单日视图：前后天导航 -->
        <div v-else class="flex items-center gap-1">
          <Button
            variant="ghost"
            size="icon"
            aria-label="前一天"
            @click="store.setDate(addDaysStr(store.currentDate, -1))"
          >
            <ChevronLeft class="h-4 w-4" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            aria-label="后一天"
            @click="store.setDate(addDaysStr(store.currentDate, 1))"
          >
            <ChevronRight class="h-4 w-4" />
          </Button>
          <span class="ml-1 flex items-center gap-2 text-sm font-medium">
            <CalendarDays class="h-4 w-4 text-muted-foreground" />
            {{ fmtDateCn(store.currentDate) }}
            <Button
              v-if="!isToday(store.currentDate)"
              variant="outline"
              size="sm"
              class="h-6 px-2 text-xs"
              @click="store.setDate(todayStr())"
            >
              回到今天
            </Button>
          </span>
        </div>
        <!-- 导出锚定单日口径，范围模式下隐藏 -->
        <div v-if="!store.dateRange" class="flex items-center gap-2">
          <DropdownMenu>
            <DropdownMenuTrigger as-child>
              <Button variant="outline" size="sm" class="gap-1">
                <Download class="h-3.5 w-3.5" />导出
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent align="end">
              <DropdownMenuLabel>导出 Markdown 文件</DropdownMenuLabel>
              <DropdownMenuSeparator />
              <DropdownMenuItem @select="exportReport('daily')">
                当日日报（{{ store.currentDate }}）
              </DropdownMenuItem>
              <DropdownMenuItem @select="exportReport('weekly')">
                本周周报（周一 ~ 周日）
              </DropdownMenuItem>
              <DropdownMenuItem @select="exportReport('monthly')">本月月报</DropdownMenuItem>
              <DropdownMenuItem @select="exportReport('all')">全部记录</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          <Button size="sm" class="gap-1" @click="createRecord">
            <Plus class="h-4 w-4" />新建记录
            <kbd class="ml-1 hidden rounded bg-primary-foreground/20 px-1 text-[10px] lg:inline">
              {{ modLabel }}+N
            </kbd>
          </Button>
        </div>
      </header>

      <div class="flex min-h-0 flex-1">
        <!-- 记录列表：单日扁平 / 范围分组 -->
        <div class="flex w-[300px] shrink-0 flex-col border-r">
          <div
            class="flex h-8 shrink-0 items-center justify-between border-b px-3 text-xs text-muted-foreground"
          >
            <span>共 {{ store.filteredRecords.length }} 条</span>
            <span v-if="store.tagFilter">标签筛选：{{ store.tagFilter }}</span>
          </div>
          <div class="min-h-0 flex-1 space-y-1.5 overflow-y-auto overflow-x-hidden p-2">
            <template v-if="store.loading">
              <div v-for="i in 3" :key="i" class="space-y-2 rounded-md border p-3">
                <Skeleton class="h-4 w-3/4" />
                <Skeleton class="h-3 w-1/2" />
              </div>
            </template>
            <!-- 范围视图：按日期分组（置顶记录归入各自日期组） -->
            <template v-else-if="store.dateRange && store.filteredRecords.length > 0">
              <section v-for="g in rangeGroups" :key="g.key" class="space-y-1.5">
                <h3 class="flex items-center gap-1 px-1 pt-1 text-xs text-muted-foreground">
                  {{ fmtDateCn(g.title) }}
                  <span class="text-[10px] opacity-70">{{ g.items.length }} 条</span>
                </h3>
                <RecordCard
                  v-for="r in g.items"
                  :key="r.id"
                  :record="r"
                  :selected="r.id === selectedId"
                  @select="selectedId = r.id"
                  @toggle-pinned="togglePinned(r.id)"
                  @request-delete="deleteTarget = r; deleteDialogOpen = true"
                />
              </section>
            </template>
            <!-- 单日视图：扁平列表（置顶已由后端排序置前） -->
            <template v-else-if="store.filteredRecords.length > 0">
              <RecordCard
                v-for="r in store.filteredRecords"
                :key="r.id"
                :record="r"
                :selected="r.id === selectedId"
                @select="selectedId = r.id"
                @toggle-pinned="togglePinned(r.id)"
                @request-delete="deleteTarget = r; deleteDialogOpen = true"
              />
            </template>
            <div
              v-else
              class="flex flex-col items-center gap-2 px-4 py-10 text-center text-sm text-muted-foreground"
            >
              <p>
                {{
                  store.tagFilter
                    ? "该标签下暂无记录"
                    : store.dateRange
                      ? "该范围内暂无记录"
                      : "这一天还没有工作记录"
                }}
              </p>
              <Button
                v-if="!store.tagFilter && !store.dateRange"
                variant="outline"
                size="sm"
                class="gap-1"
                @click="createRecord"
              >
                <Plus class="h-4 w-4" />新建第一条记录
              </Button>
            </div>
          </div>
        </div>

        <!-- 新建/编辑表单 -->
        <div class="min-w-0 flex-1 overflow-y-auto overflow-x-hidden p-4">
          <RecordEditor
            ref="editorRef"
            :record="selectedRecord"
            @saved="onSaved"
            @deleted="onDeleted"
            @create="createRecord"
            @toggle-pinned="selectedRecord && togglePinned(selectedRecord.id)"
          />
        </div>
      </div>
    </section>

    <!-- 列表右键删除确认（受控弹窗；取消仅关弹窗，不清删除目标） -->
    <AlertDialog
      :open="deleteDialogOpen"
      @update:open="(v: boolean) => { deleteDialogOpen = v }"
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>移入回收站？</AlertDialogTitle>
          <AlertDialogDescription>
            「{{ deleteTarget?.title || "（无标题）" }}」删除后可在回收站中恢复，不会立即丢失数据。
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>取消</AlertDialogCancel>
          <AlertDialogAction :disabled="deletingFromList" @click="confirmListDelete">
            删除
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>

    <!-- 标签弹窗视图：展示标签下全部记录（跨日期），点击记录跳转选中 -->
    <TagRecordsDialog
      :open="tagDialog.open"
      :tag="tagDialog.tag"
      @close="tagDialog.open = false"
      @jump="onTagJump"
    />
  </div>
</template>
