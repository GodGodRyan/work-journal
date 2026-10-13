/**
 * 全局状态（Pinia）：当前选中日期/日期范围、主题、标签缓存、记录列表、
 * 以及 App 全局快捷键 → 页面动作的跨组件信号。
 * 注意：业务数据只存 SQLite（Rust 侧），localStorage 仅保存主题与界面偏好。
 */
import { defineStore } from "pinia";
import * as api from "@/api/records";
import { todayStr } from "@/composables/useDate";
import type { TagStat, ThemeMode, WorkRecord } from "@/types/work-record";

const THEME_KEY = "work-journal-theme";
const RANGE_SELECT_KEY = "work-journal-range-select";

function systemPrefersDark(): boolean {
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

export const useAppStore = defineStore("app", {
  state: () => ({
    /** 当前选中日期 yyyy-MM-dd，默认今天 */
    currentDate: todayStr(),
    /** 日历划选日期范围（null = 单日视图） */
    dateRange: null as { from: string; to: string } | null,
    /** 日历范围选择开关（localStorage 持久化 UI 偏好，默认开启） */
    rangeSelectEnabled: localStorage.getItem(RANGE_SELECT_KEY) !== "0",
    /** 主题模式（localStorage 持久化 UI 偏好，非业务数据） */
    theme: (localStorage.getItem(THEME_KEY) as ThemeMode | null) ?? "system",
    /** 当前标签筛选（null = 不筛选） */
    tagFilter: null as string | null,
    /** 标签缓存 */
    tags: [] as TagStat[],
    /** 有记录的日期集合（日历高亮） */
    recordDates: [] as string[],
    /** 当前日期/日期范围的记录列表 */
    records: [] as WorkRecord[],
    /** 全部置顶记录（侧边栏置顶区固定展示） */
    pinnedRecords: [] as WorkRecord[],
    loading: false,
    // 跨组件信号：App 快捷键 → 对应页面动作（自增计数触发 watch）
    newRecordSignal: 0,
    searchFocusSignal: 0,
    manualSaveSignal: 0,
  }),
  getters: {
    /** 实际是否深色（system 模式跟随系统） */
    isDark(): boolean {
      return this.theme === "dark" || (this.theme === "system" && systemPrefersDark());
    },
    recordDateSet(): Set<string> {
      return new Set(this.recordDates);
    },
    /** 按当前标签筛选后的记录（单日与范围视图共用） */
    filteredRecords(): WorkRecord[] {
      if (!this.tagFilter) return this.records;
      const f = this.tagFilter;
      return this.records.filter((r) =>
        r.tags
          .split(/[,，;；]/)
          .map((t) => t.trim())
          .includes(f),
      );
    },
  },
  actions: {
    /** 应用主题到 <html>（shadcn-vue 深色模式基于 .dark class） */
    applyTheme(): void {
      document.documentElement.classList.toggle("dark", this.isDark);
    },
    /** 启动初始化：应用主题并监听系统主题变化（仅 system 模式生效） */
    initTheme(): void {
      this.applyTheme();
      window
        .matchMedia("(prefers-color-scheme: dark)")
        .addEventListener("change", () => {
          if (this.theme === "system") this.applyTheme();
        });
    },
    setTheme(t: ThemeMode): void {
      this.theme = t;
      localStorage.setItem(THEME_KEY, t);
      this.applyTheme();
    },
    /** 切换单日（清空日期范围与标签筛选） */
    setDate(d: string): void {
      this.currentDate = d;
      this.dateRange = null;
      this.tagFilter = null;
    },
    /** 进入范围视图（currentDate 置为范围起点，清空标签筛选） */
    setRange(from: string, to: string): void {
      this.dateRange = { from, to };
      this.currentDate = from;
      this.tagFilter = null;
    },
    /** 退出范围视图回到单日（保持 currentDate 不变） */
    exitRange(): void {
      this.dateRange = null;
    },
    /** 日历范围选择开关（持久化；关闭时若正处于范围视图则退回单日） */
    setRangeSelectEnabled(v: boolean): void {
      this.rangeSelectEnabled = v;
      localStorage.setItem(RANGE_SELECT_KEY, v ? "1" : "0");
      if (!v && this.dateRange) this.exitRange();
    },
    setTagFilter(tag: string | null): void {
      this.tagFilter = tag;
    },
    async loadRecords(): Promise<void> {
      this.loading = true;
      try {
        this.records = this.dateRange
          ? await api.listByRange(this.dateRange.from, this.dateRange.to)
          : await api.listByDate(this.currentDate);
      } finally {
        this.loading = false;
      }
    },
    async loadTags(): Promise<void> {
      this.tags = await api.allTags();
    },
    async loadRecordDates(): Promise<void> {
      this.recordDates = await api.recordDates();
    },
    async loadPinned(): Promise<void> {
      this.pinnedRecords = await api.listPinned();
    },
    /** 加载侧边栏数据（标签 / 日历标记 / 置顶区） */
    async loadSidebar(): Promise<void> {
      await Promise.all([this.loadTags(), this.loadRecordDates(), this.loadPinned()]);
    },
    /** 数据变更后的全量刷新（新建/删除/恢复备份等） */
    async refreshAll(): Promise<void> {
      await Promise.all([
        this.loadRecords(),
        this.loadTags(),
        this.loadRecordDates(),
        this.loadPinned(),
      ]);
    },
    /** 恢复备份后的全量重载 */
    async reloadAll(): Promise<void> {
      await this.refreshAll();
    },
    // ---- 快捷键信号 ----
    requestNewRecord(): void {
      this.newRecordSignal++;
    },
    requestSearchFocus(): void {
      this.searchFocusSignal++;
    },
    requestManualSave(): void {
      this.manualSaveSignal++;
    },
  },
});
