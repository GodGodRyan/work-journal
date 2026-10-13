/**
 * 全局快捷键（跟随系统：macOS 用 ⌘，Windows/Linux 用 Ctrl）：
 * - N：新建工作记录
 * - F：聚焦搜索输入框
 * - S：手动保存（自动保存的兜底）
 * - ,（逗号）：打开设置页
 */
import { onMounted, onUnmounted } from "vue";

/** 是否 macOS（WKWebView/浏览器下用 userAgent 判断，platform 作兜底） */
export const isMac: boolean =
  typeof navigator !== "undefined" &&
  (/mac/i.test(navigator.userAgent) || /mac/i.test(navigator.platform || ""));

/** 主修饰键显示名：macOS ⌘ / 其余 Ctrl */
export const modLabel: string = isMac ? "⌘" : "Ctrl";

/** 事件主修饰键是否按下（跟随系统：macOS 认 Cmd，其余认 Ctrl） */
export function isModPressed(e: KeyboardEvent): boolean {
  return isMac ? e.metaKey : e.ctrlKey;
}

export interface HotkeyHandlers {
  onNew?: () => void;
  onSearch?: () => void;
  onSave?: () => void;
  onSettings?: () => void;
}

export function useHotkeys(handlers: HotkeyHandlers): void {
  const onKey = (e: KeyboardEvent): void => {
    if (!isModPressed(e) || e.altKey) return;
    const k = e.key.toLowerCase();
    if (k === "n") {
      e.preventDefault();
      handlers.onNew?.();
    } else if (k === "f") {
      e.preventDefault();
      handlers.onSearch?.();
    } else if (k === "s") {
      e.preventDefault();
      handlers.onSave?.();
    } else if (k === ",") {
      e.preventDefault();
      handlers.onSettings?.();
    }
  };
  onMounted(() => window.addEventListener("keydown", onKey));
  onUnmounted(() => window.removeEventListener("keydown", onKey));
}
