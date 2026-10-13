<script setup lang="ts">
/**
 * 搜索页面：FTS5 全文搜索（标题/内容/标签），关键词高亮，
 * 支持限定日期范围（两个日历 Popover），300ms 防抖。
 * ⌘/Ctrl+F 全局快捷键可随时聚焦本页输入框（跟随系统）。
 */
import { computed, onMounted, ref, shallowRef, watch } from "vue";
import type { DateValue } from "reka-ui";
import { CalendarDays, Search, X } from "@lucide/vue";
import { Button } from "@/components/ui/button";
import { Calendar } from "@/components/ui/calendar";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { Skeleton } from "@/components/ui/skeleton";
import { toast } from "@/components/ui/toast";
import SearchResultItem from "@/components/SearchResultItem.vue";
import * as api from "@/api/records";
import { useAppStore } from "@/stores/app";
import { errMsg } from "@/lib/utils";
import { dateValueToStr, fmtDate, parseDateStr } from "@/composables/useDate";
import { modLabel } from "@/composables/useHotkeys";
import type { WorkRecord } from "@/types/work-record";

const store = useAppStore();

const query = ref("");
const from = ref<Date | null>(null);
const to = ref<Date | null>(null);
const results = ref<WorkRecord[]>([]);

/** 命中词列表（按空白拆分），供结果项前端高亮 */
const terms = computed(() =>
  query.value.trim().split(/\s+/).filter((t) => t.length > 0),
);
const loading = ref(false);
const searched = ref(false);

// 日期 Popover 控制。
// fromTmp/toTmp 用 shallowRef 避免 UnwrapRef 展开 DateValue 名义类型（TS2322），
// 宽类型同时兼容 reka-ui 单选/多选两种 update:modelValue 载荷。
const fromOpen = ref(false);
const toOpen = ref(false);
const fromTmp = shallowRef<DateValue | DateValue[] | null | undefined>(undefined);
const toTmp = shallowRef<DateValue | DateValue[] | null | undefined>(undefined);

// 聚焦目标：组件根 div 中的 input
const searchBox = ref<HTMLDivElement | null>(null);

let debounce: ReturnType<typeof setTimeout> | undefined;

async function doSearch(): Promise<void> {
  const q = query.value.trim();
  searched.value = true;
  if (!q) {
    results.value = [];
    return;
  }
  loading.value = true;
  try {
    results.value = await api.search(
      q,
      from.value ? fmtDate(from.value) : null,
      to.value ? fmtDate(to.value) : null,
    );
  } catch (e) {
    toast({ title: "搜索失败", description: errMsg(e), variant: "destructive" });
    results.value = [];
  } finally {
    loading.value = false;
  }
}

function scheduleSearch(): void {
  if (debounce) clearTimeout(debounce);
  debounce = setTimeout(() => {
    void doSearch();
  }, 300);
}

watch([query, from, to], scheduleSearch);

function clearRange(): void {
  from.value = null;
  to.value = null;
  fromTmp.value = undefined;
  toTmp.value = undefined;
}

// 日历选中即收拢 Popover。
// DateValue（CalendarDate 等）不是原生 Date，需经 dateValueToStr 转 yyyy-MM-dd 再 parseDateStr 成 Date
watch(fromTmp, (d) => {
  if (d && !Array.isArray(d)) {
    from.value = parseDateStr(dateValueToStr(d));
    fromOpen.value = false;
  }
});
watch(toTmp, (d) => {
  if (d && !Array.isArray(d)) {
    to.value = parseDateStr(dateValueToStr(d));
    toOpen.value = false;
  }
});

function focusInput(): void {
  searchBox.value?.querySelector("input")?.focus();
}

onMounted(() => {
  focusInput();
});

// 快捷键信号（跟随系统）：聚焦搜索输入框
watch(
  () => store.searchFocusSignal,
  () => {
    focusInput();
  },
);
</script>

<template>
  <div class="flex h-full flex-col">
    <header class="flex h-14 shrink-0 items-center justify-between border-b px-4">
      <h1 class="text-sm font-semibold">搜索</h1>
      <span class="text-xs text-muted-foreground">全文检索标题 / 内容 / 标签 · {{ modLabel }}+F 快速聚焦</span>
    </header>

    <div class="min-h-0 flex-1 overflow-y-auto p-4">
      <div class="mx-auto max-w-3xl space-y-4">
        <!-- 搜索区 -->
        <div class="space-y-2">
          <div ref="searchBox" class="relative">
            <Search class="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              v-model="query"
              type="search"
              placeholder="输入关键词，如：登录接口 / 修复 / 后端…"
              class="h-10 pl-9 pr-9"
              @keyup.enter="doSearch"
            />
            <button
              v-if="query"
              type="button"
              class="absolute right-2 top-1/2 -translate-y-1/2 rounded p-1 text-muted-foreground hover:text-foreground"
              aria-label="清空关键词"
              @click="query = ''"
            >
              <X class="h-4 w-4" />
            </button>
          </div>

          <!-- 日期范围 -->
          <div class="flex flex-wrap items-center gap-2 text-sm">
            <Popover v-model:open="fromOpen">
              <PopoverTrigger as-child>
                <Button variant="outline" size="sm" class="gap-1 font-normal">
                  <CalendarDays class="h-3.5 w-3.5" />
                  {{ from ? fmtDate(from) : "开始日期" }}
                </Button>
              </PopoverTrigger>
              <PopoverContent class="w-auto p-0" align="start">
                <Calendar v-model="fromTmp" />
              </PopoverContent>
            </Popover>
            <span class="text-muted-foreground">~</span>
            <Popover v-model:open="toOpen">
              <PopoverTrigger as-child>
                <Button variant="outline" size="sm" class="gap-1 font-normal">
                  <CalendarDays class="h-3.5 w-3.5" />
                  {{ to ? fmtDate(to) : "结束日期" }}
                </Button>
              </PopoverTrigger>
              <PopoverContent class="w-auto p-0" align="start">
                <Calendar v-model="toTmp" />
              </PopoverContent>
            </Popover>
            <Button
              v-if="from || to"
              variant="ghost"
              size="sm"
              class="h-7 gap-1 text-xs text-muted-foreground"
              @click="clearRange"
            >
              <X class="h-3 w-3" />清除范围
            </Button>
          </div>
        </div>

        <!-- 结果 -->
        <div class="space-y-1.5">
          <p v-if="searched && query.trim()" class="text-xs text-muted-foreground">
            共 {{ results.length }} 条结果
          </p>
          <template v-if="loading">
            <div v-for="i in 3" :key="i" class="space-y-2 rounded-md border p-3">
              <Skeleton class="h-4 w-1/2" />
              <Skeleton class="h-3 w-full" />
            </div>
          </template>
          <template v-else-if="results.length > 0">
            <SearchResultItem v-for="h in results" :key="h.id" :record="h" :terms="terms" />
          </template>
          <p
            v-else-if="searched && query.trim()"
            class="py-12 text-center text-sm text-muted-foreground"
          >
            没有匹配的记录，换个关键词试试
          </p>
          <p v-else class="py-12 text-center text-sm text-muted-foreground">
            输入关键词开始搜索，匹配标题、内容与标签
          </p>
        </div>
      </div>
    </div>
  </div>
</template>
