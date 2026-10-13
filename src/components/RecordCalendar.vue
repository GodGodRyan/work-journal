<script setup lang="ts">
/**
 * 左侧日历（范围选择）：由 reka-ui RangeCalendar 原语组合而成。
 * - 点选单日 → 立即切换 store.currentDate（单日视图）
 * - 点选两个日期 → store.dateRange（主页列表切换为范围视图）
 * - 有记录的日期下方显示小圆点标记（选中范围内自动反色）
 * - locale 传字符串 "zh-CN"（RangeCalendarRootProps.locale 为 string，直接生效）
 *
 * 交互语义（reka-ui 源码确认，与直觉不同）：
 * 首次点击 X → model = { start: X, end: undefined }（end 不是 start==end）；
 * 再点 Y → { start: min, end: max }；已有完整范围再点新日期 → 重新开始新一轮。
 * 因此 watch(model) 按三条规则处理，且必须用 internal 标志防止 store→model
 * 反向同步把 {X, undefined} 覆盖成 {X, X}（否则范围永远无法完成）。
 *
 * 注意：model 必须用 shallowRef（普通 ref 会被 Vue UnwrapRef 深度展开，
 * 破坏 DateValue/DateRange 名义类型导致 TS2322）。
 */
import { onBeforeUnmount, reactive, shallowRef, watch } from "vue";
import { CalendarRange } from "@lucide/vue";
import { parseDate } from "@internationalized/date";
import type { DateRange, DateValue } from "reka-ui";
import { RangeCalendarRoot } from "reka-ui";
import { Switch } from "@/components/ui/switch";
import {
  RangeCalendarCell,
  RangeCalendarCellTrigger,
  RangeCalendarGrid,
  RangeCalendarGridBody,
  RangeCalendarGridHead,
  RangeCalendarGridRow,
  RangeCalendarHeadCell,
  RangeCalendarHeader,
  RangeCalendarHeading,
  RangeCalendarNextButton,
  RangeCalendarPrevButton,
} from "@/components/ui/range-calendar";
import { useAppStore } from "@/stores/app";
import { dateValueToStr, fmtDateCn, fmtTime } from "@/composables/useDate";
import * as api from "@/api/records";
import type { WorkRecord } from "@/types/work-record";

const store = useAppStore();

/** 按 store 当前状态（单日或范围）构造日历 model */
function modelFromStore(): DateRange {
  if (store.dateRange) {
    return {
      start: parseDate(store.dateRange.from),
      end: parseDate(store.dateRange.to),
    };
  }
  const d = parseDate(store.currentDate);
  return { start: d, end: d };
}

const model = shallowRef<DateRange | null>(modelFromStore());

/** store→model 同步期间置位，防止双向循环 */
let internal = false;

// 日历选择 → 写回 store（三条规则，见文件头注释）
watch(model, (d) => {
  if (!d) return;
  // 范围选择已关闭：任何点选都收敛为单日（视觉与行为一致），不形成范围。
  // 已收敛时不再回写：shallowRef 按引用比较，每次赋新对象都会再次触发本 watch，
  // 无限循环会卡死主线程
  if (!store.rangeSelectEnabled) {
    const day = d.start ?? d.end;
    if (!day) return;
    const s = dateValueToStr(day);
    const converged =
      d.start !== undefined &&
      d.end !== undefined &&
      dateValueToStr(d.start) === s &&
      dateValueToStr(d.end) === s;
    if (!converged) model.value = { start: day, end: day };
    store.setDate(s);
    return;
  }
  internal = true;
  try {
    if (d.start && d.end) {
      const from = dateValueToStr(d.start);
      const to = dateValueToStr(d.end);
      if (from === to) {
        store.setDate(from); // 单选同一天：单日视图
      } else if (from < to) {
        store.setRange(from, to); // 范围视图（reka 保证 start <= end，此处防御性排序）
      } else {
        store.setRange(to, from);
      }
    } else if (d.start) {
      // 只点了起始（end 未定）：立即切单日视图，保留日历 in-progress 状态
      store.setDate(dateValueToStr(d.start));
    } else {
      // 选择被取消：把日历高亮同步回 store 当前状态
      model.value = modelFromStore();
    }
  } finally {
    // internal 复位必须放宏任务：store→model 的 watch 回调是 Vue 异步 flush，
    // 执行时机在本回调返回之后；若同步复位，反向同步会把 in-progress 的
    // {start, undefined} 回写成 {start, start}，导致范围永远无法完成
    setTimeout(() => {
      internal = false;
    });
  }
});

// store 变更（日期导航按钮 / 退出范围 / 时间线跳转等）→ 同步日历高亮。
// 组合键保证「退出范围时 currentDate 不变」也能触发（dateRange 由 null 变非 null 或反之）。
watch(
  () =>
    store.dateRange
      ? `${store.dateRange.from}~${store.dateRange.to}`
      : store.currentDate,
  () => {
    if (internal) return;
    model.value = modelFromStore();
  },
);

function hasRecords(d: DateValue): boolean {
  return store.recordDateSet.has(dateValueToStr(d));
}

// ---- 悬浮预览：有记录的日期悬停显示最近 N 条 ----
// 三重防护防请求风暴：250ms 延迟（快速划过不查询）+ 会话内缓存 + 竞态 token。
const HOVER_DELAY_MS = 250;
const HOVER_LIMIT = 10;
const HOVER_POP_W = 280; // 浮层估算宽度（含边框），定位翻转用
const HOVER_POP_H = 300; // 浮层最大高度（含标题），定位翻转用

const hover = reactive({
  visible: false,
  date: "",
  x: 0,
  y: 0,
  records: [] as WorkRecord[],
  loading: false,
});

/** 会话内缓存（新建/编辑后可能过期，重新悬停刷新即可，属可接受的非关键路径） */
const hoverCache = new Map<string, WorkRecord[]>();
let hoverTimer: ReturnType<typeof setTimeout> | null = null;
let hoverToken = 0; // 每次进入 +1；查询返回后 token 不匹配则丢弃（已离开或换了格子）

function onCellEnter(d: DateValue, e: MouseEvent): void {
  const date = dateValueToStr(d);
  if (!store.recordDateSet.has(date)) return;
  const token = ++hoverToken;
  const { clientX, clientY } = e;
  if (hoverTimer) clearTimeout(hoverTimer);
  hoverTimer = setTimeout(() => {
    void loadHover(date, token, clientX, clientY);
  }, HOVER_DELAY_MS);
}

function onCellLeave(): void {
  hoverToken++; // 在途查询过期
  if (hoverTimer) {
    clearTimeout(hoverTimer);
    hoverTimer = null;
  }
  hover.visible = false;
}

async function loadHover(
  date: string,
  token: number,
  cx: number,
  cy: number,
): Promise<void> {
  let records = hoverCache.get(date);
  if (!records) {
    hover.date = date;
    hover.loading = true;
    hover.visible = true;
    positionPop(cx, cy);
    try {
      records = await api.recentByDate(date, HOVER_LIMIT);
    } catch {
      // 悬浮预览属非关键路径：查询失败静默隐藏，不打扰用户
      if (token === hoverToken) hover.visible = false;
      return;
    }
    if (!Array.isArray(records)) {
      // 异常返回（理论上命令成功必为数组，防御性兜底）：同样静默隐藏
      if (token === hoverToken) hover.visible = false;
      return;
    }
    if (token !== hoverToken) return;
    hoverCache.set(date, records);
  }
  hover.date = date;
  hover.records = records;
  hover.loading = false;
  positionPop(cx, cy);
  hover.visible = true;
}

/** 浮层跟随鼠标右下；近右/下边缘时钳制进视口（整体翻转） */
function positionPop(cx: number, cy: number): void {
  hover.x = Math.min(cx + 12, window.innerWidth - HOVER_POP_W - 8);
  hover.y = Math.min(cy + 12, Math.max(8, window.innerHeight - HOVER_POP_H - 8));
}

onBeforeUnmount(() => {
  if (hoverTimer) clearTimeout(hoverTimer);
});
</script>

<template>
  <div>
    <!-- 范围选择开关：关闭后日历只做单日切换（不形成日期范围），若正处于范围视图则自动退回单日 -->
    <div class="flex items-center justify-between px-3 pt-2">
      <span class="flex items-center gap-1.5 text-[11px] text-muted-foreground">
        <CalendarRange class="h-3.5 w-3.5" />范围选择
      </span>
      <Switch
        :model-value="store.rangeSelectEnabled"
        :aria-label="store.rangeSelectEnabled ? '关闭范围选择' : '开启范围选择'"
        class="scale-[0.85]"
        @update:model-value="store.setRangeSelectEnabled"
      />
    </div>
    <!-- paged-navigation：恒单月网格。侧边栏 248px 放不下 reka 跨月自动展开的多个月网格，
         跨月范围靠翻页选择（先点起始，翻月，再点结束），网格内容不会超出容器。
         select-none：禁用按住拖动的文本划选高亮（选择范围仍靠点选起止日期） -->
    <RangeCalendarRoot
      v-slot="{ grid, weekDays }"
      v-model="model"
      locale="zh-CN"
      paged-navigation
      class="w-full select-none p-2"
    >
      <RangeCalendarHeader>
        <RangeCalendarPrevButton />
        <RangeCalendarHeading class="text-sm font-medium" />
        <RangeCalendarNextButton />
      </RangeCalendarHeader>

      <div class="mt-2 flex flex-col gap-4">
        <RangeCalendarGrid v-for="month in grid" :key="month.value.toString()">
          <RangeCalendarGridHead>
            <RangeCalendarGridRow>
              <RangeCalendarHeadCell
                v-for="day in weekDays"
                :key="day"
                class="w-7 rounded-md text-[0.8rem] font-normal text-muted-foreground"
              >
                {{ day }}
              </RangeCalendarHeadCell>
            </RangeCalendarGridRow>
          </RangeCalendarGridHead>
          <RangeCalendarGridBody>
            <RangeCalendarGridRow
              v-for="(weekDates, index) in month.rows"
              :key="`week-${index}`"
              class="mt-1 w-full"
            >
              <RangeCalendarCell
                v-for="weekDate in weekDates"
                :key="weekDate.toString()"
                :date="weekDate"
                class="group relative"
                @mouseenter="onCellEnter(weekDate, $event)"
                @mouseleave="onCellLeave"
              >
                <RangeCalendarCellTrigger :day="weekDate" :month="month.value">
                  {{ weekDate.day }}
                </RangeCalendarCellTrigger>
                <!-- 有记录日期的小圆点。仅在深色端点（selection-start/end）上反色；
                     范围内中间日期是浅色 accent 背景，圆点保持深色才可见 -->
                <span
                  v-if="hasRecords(weekDate)"
                  class="pointer-events-none absolute bottom-0.5 left-1/2 h-1 w-1 -translate-x-1/2 rounded-full bg-primary group-has-[[data-selection-start]]:bg-primary-foreground group-has-[[data-selection-end]]:bg-primary-foreground"
                />
              </RangeCalendarCell>
            </RangeCalendarGridRow>
          </RangeCalendarGridBody>
        </RangeCalendarGrid>
      </div>
    </RangeCalendarRoot>

    <!-- 悬浮预览浮层：pointer-events-none 避免浮层挡鼠标引发 mouseleave 抖动 -->
    <Teleport to="body">
      <div
        v-if="hover.visible"
        class="pointer-events-none fixed z-50 w-[272px] rounded-md border bg-popover p-2 shadow-md"
        :style="{ left: `${hover.x}px`, top: `${hover.y}px` }"
      >
        <p class="mb-1 px-1 text-[11px] font-medium text-muted-foreground">
          {{ fmtDateCn(hover.date) }} 最近 {{ hover.records.length }} 条
        </p>
        <p v-if="hover.loading" class="px-1 py-2 text-xs text-muted-foreground">加载中…</p>
        <ul v-else class="max-h-[240px] space-y-1 overflow-y-auto">
          <li
            v-for="r in hover.records"
            :key="r.id"
            class="flex items-baseline justify-between gap-2 rounded px-1 py-0.5 text-xs"
          >
            <span class="truncate">{{ r.title || "（无标题）" }}</span>
            <span class="shrink-0 tabular-nums text-[10px] text-muted-foreground">
              {{ fmtTime(r.updated_at) }}
            </span>
          </li>
        </ul>
      </div>
    </Teleport>
  </div>
</template>
