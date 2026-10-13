/**
 * 编辑防抖自动保存。
 * markDirty：内容变更后调用，500ms 防抖后落库；
 * flush：立即保存（手动保存按钮 / 切换记录前 / 组件卸载前兜底）；
 * 保存失败不吞错：通过 onError 回调由界面 toast 提示。
 */
import { onBeforeUnmount, ref } from "vue";

export interface AutoSaveOptions {
  /** 执行保存（失败抛异常） */
  save: () => Promise<unknown>;
  /** 防抖延迟 ms，默认 500 */
  delay?: number;
  /** 保存失败回调 */
  onError?: (e: unknown) => void;
}

export function useAutoSave(options: AutoSaveOptions) {
  const delay = options.delay ?? 500;
  /** 有未落盘的修改 */
  const pending = ref(false);
  /** 正在保存中 */
  const saving = ref(false);
  let dirty = false;
  let timer: ReturnType<typeof setTimeout> | undefined;

  function markDirty(): void {
    dirty = true;
    pending.value = true;
    if (timer !== undefined) clearTimeout(timer);
    timer = setTimeout(() => {
      void flush();
    }, delay);
  }

  /** 立即保存；返回是否成功。无未落盘内容时直接返回 true。 */
  async function flush(): Promise<boolean> {
    if (timer !== undefined) {
      clearTimeout(timer);
      timer = undefined;
    }
    if (!dirty) return true;
    saving.value = true;
    try {
      await options.save();
      dirty = false;
      pending.value = false;
      return true;
    } catch (e) {
      // 保存失败保持 dirty 状态，等待下次触发重试，不静默丢失
      options.onError?.(e);
      return false;
    } finally {
      saving.value = false;
    }
  }

  // 组件卸载前兜底保存最后内容
  onBeforeUnmount(() => {
    void flush();
  });

  return { pending, saving, markDirty, flush };
}
