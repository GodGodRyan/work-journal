/**
 * 媒体块复制缓存：选中附件媒体块时（NodeView selected）预取 data URL，
 * Ctrl+C 的 copy 事件是同步的（事件结束后剪贴板数据即冻结），
 * 不能异步获取，故用缓存兜底。缓存未命中时仅复制 HTML（粘贴回编辑器仍可还原）。
 */
import type { Attachment } from "@/types/work-record";
import * as api from "@/api/records";

interface CachedCopy {
  dataUrl: string;
  name: string;
}

export const attachCopyCache = new Map<number, CachedCopy>();

/** 预取附件 data URL 进复制缓存（已缓存则跳过；失败静默，复制时降级为 HTML） */
export async function prefetchAttachCopy(a: Attachment): Promise<void> {
  if (attachCopyCache.has(a.id)) return;
  try {
    attachCopyCache.set(a.id, {
      dataUrl: await api.getAttachmentData(a.id),
      name: a.file_name,
    });
  } catch {
    // 预取失败不打断编辑；复制时缓存未命中则仅复制 HTML
  }
}

/** data URL → File 对象（写入剪贴板 items 用） */
export function dataUrlToFile(dataUrl: string, name: string): File {
  const m = dataUrl.match(/^data:([^;]+);base64,(.+)$/);
  if (!m) throw new Error("无效的 data URL");
  const mime = m[1];
  const bin = atob(m[2]);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return new File([bytes], name, { type: mime });
}
