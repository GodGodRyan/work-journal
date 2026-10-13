/**
 * 日期工具：统一 yyyy-MM-dd 字符串与 Date 的转换、中文星期、月统计键。
 */
import { addDays, format, parse } from "date-fns";

export const DATE_FMT = "yyyy-MM-dd";

const WEEK_CN = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"] as const;

/** 今天 yyyy-MM-dd */
export function todayStr(): string {
  return format(new Date(), DATE_FMT);
}

/** yyyy-MM-dd → Date */
export function parseDateStr(s: string): Date {
  return parse(s, DATE_FMT, new Date());
}

/** Date | yyyy-MM-dd → yyyy-MM-dd */
export function fmtDate(d: Date | string): string {
  return typeof d === "string" ? d : format(d, DATE_FMT);
}

/** 中文星期（0=周日） */
export function weekdayCn(d: Date | string): string {
  const date = typeof d === "string" ? parseDateStr(d) : d;
  return WEEK_CN[date.getDay()];
}

/** 2026-10-09（周四） */
export function fmtDateCn(s: string): string {
  return `${s}（${weekdayCn(s)}）`;
}

/** 日期加减天数 */
export function addDaysStr(s: string, n: number): string {
  return format(addDays(parseDateStr(s), n), DATE_FMT);
}

/** 时间戳（yyyy-MM-dd HH:mm:ss）→ MM-dd HH:mm 短显示 */
export function fmtTime(ts: string): string {
  return ts.length >= 16 ? ts.slice(5, 16) : ts;
}

/** 是否今天 */
export function isToday(s: string): boolean {
  return s === todayStr();
}

/**
 * reka-ui 国际化日期对象（CalendarDate/ZonedDateTime 等，均有 year/month/day）→ yyyy-MM-dd。
 * 不能传原生 Date（无这些字段）。
 */
export function dateValueToStr(d: { year: number; month: number; day: number }): string {
  const mm = String(d.month).padStart(2, "0");
  const dd = String(d.day).padStart(2, "0");
  return `${d.year}-${mm}-${dd}`;
}
