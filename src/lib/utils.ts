import { type ClassValue, clsx } from "clsx";
import { twMerge } from "tailwind-merge";

/** 合并 Tailwind 类名（shadcn-vue 标准工具函数） */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/** 转义正则特殊字符（用户查询词可能包含） */
function escapeRegExp(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** 首个命中词在文本中的位置（无命中返回 -1） */
function firstHitIndex(text: string, terms: string[]): number {
  let idx = -1;
  for (const t of terms) {
    const i = text.indexOf(t);
    if (i >= 0 && (idx < 0 || i < idx)) idx = i;
  }
  return idx;
}

/**
 * 按命中词把文本切为片段列表 [{ text, hit }]。
 * 纯文本渲染（模板插值 + <mark>），不经过 v-html，规避 XSS；
 * 词按长度降序优先匹配，避免短词先截断长词。
 */
export function splitByTerms(
  text: string,
  terms: string[],
): { text: string; hit: boolean }[] {
  if (terms.length === 0) return [{ text, hit: false }];
  const sorted = [...terms].sort((a, b) => b.length - a.length);
  const re = new RegExp(sorted.map(escapeRegExp).join("|"), "g");
  const out: { text: string; hit: boolean }[] = [];
  let last = 0;
  for (const m of text.matchAll(re)) {
    const idx = m.index ?? 0;
    if (idx > last) out.push({ text: text.slice(last, idx), hit: false });
    out.push({ text: m[0], hit: true });
    last = idx + m[0].length;
  }
  if (last < text.length) out.push({ text: text.slice(last), hit: false });
  return out;
}

/** 内容摘要：截取首个命中词前后各 radius 字符（无命中时取开头） */
export function snippetAroundTerm(text: string, terms: string[], radius = 40): string {
  const idx = firstHitIndex(text, terms);
  if (idx < 0) {
    return text.length > radius * 2 ? `${text.slice(0, radius * 2)}…` : text;
  }
  const start = Math.max(0, idx - radius);
  const end = Math.min(text.length, idx + radius);
  return `${start > 0 ? "…" : ""}${text.slice(start, end)}${end < text.length ? "…" : ""}`;
}

/** 拆分逗号分隔的标签字符串为数组 */
export function splitTags(tags: string): string[] {
  return tags
    .split(/[,，;；]/)
    .map((t) => t.trim())
    .filter((t) => t.length > 0);
}

/** 拼接标签数组为逗号分隔字符串（去重） */
export function joinTags(tags: string[]): string {
  return [...new Set(tags.map((t) => t.trim()).filter((t) => t.length > 0))].join(",");
}

/** 从 Tauri invoke 异常中提取可读错误信息 */
export function errMsg(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}

/** 内容中的附件占位符 [附件:123] 转为可读文本（列表摘要/搜索片段等纯文本展示场景） */
export function maskAttachmentMarkers(s: string): string {
  return s.replace(/\[附件:\d+\]/g, "📎 附件");
}
