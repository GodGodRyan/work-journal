<script setup lang="ts">
/**
 * 标签弹窗视图：点击侧边栏标签弹出，展示该标签的全部记录（跨日期，按日期倒序）。
 * 手写 Teleport 覆盖层（项目无现成 Dialog 组件，沿用右键菜单的 Teleport 模式）；
 * 点击记录 emit jump，由主页复用 jumpToPinned 模式跳转选中。
 * Esc / 点击遮罩 / 关闭按钮均可关闭。
 */
import { onBeforeUnmount, onMounted, ref, watch } from "vue";
import { Tag, X } from "@lucide/vue";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import * as api from "@/api/records";
import { fmtDateCn, fmtTime } from "@/composables/useDate";
import { errMsg } from "@/lib/utils";
import type { WorkRecord } from "@/types/work-record";

/** 与后端 clamp 上限一致；命中上限时提示「仅显示前 N 条」 */
const MAX_LIST = 500;

const props = defineProps<{
  open: boolean;
  tag: string;
}>();

const emit = defineEmits<{
  close: [];
  jump: [r: WorkRecord];
}>();

const records = ref<WorkRecord[]>([]);
const loading = ref(false);
const error = ref("");

// 打开时按当前标签加载（关闭不触发）
watch(
  () => [props.open, props.tag],
  async ([open]) => {
    if (!open) return;
    loading.value = true;
    error.value = "";
    records.value = [];
    try {
      records.value = await api.recordsByTag(props.tag, MAX_LIST);
    } catch (e) {
      error.value = errMsg(e);
    } finally {
      loading.value = false;
    }
  },
  { immediate: true },
);

function onKeydown(e: KeyboardEvent): void {
  if (e.key === "Escape" && props.open) emit("close");
}

onMounted(() => window.addEventListener("keydown", onKeydown));
onBeforeUnmount(() => window.removeEventListener("keydown", onKeydown));
</script>

<template>
  <Teleport to="body">
    <div
      v-if="props.open"
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      @click.self="emit('close')"
    >
      <div
        class="flex max-h-[70vh] w-[560px] max-w-[90vw] flex-col rounded-lg border bg-card shadow-lg"
      >
        <header class="flex h-12 shrink-0 items-center justify-between border-b px-4">
          <h2 class="flex min-w-0 items-center gap-1.5 text-sm font-semibold">
            <Tag class="h-4 w-4 shrink-0 text-primary" />
            <span class="truncate">标签：{{ props.tag }}</span>
            <span v-if="!loading" class="shrink-0 text-xs font-normal text-muted-foreground">
              {{ records.length }} 条
            </span>
          </h2>
          <Button
            variant="ghost"
            size="icon"
            class="h-7 w-7 shrink-0"
            aria-label="关闭"
            @click="emit('close')"
          >
            <X class="h-4 w-4" />
          </Button>
        </header>
        <div class="min-h-0 flex-1 overflow-y-auto p-2">
          <div v-if="loading" class="space-y-2 p-2">
            <Skeleton v-for="i in 5" :key="i" class="h-9 w-full" />
          </div>
          <p v-else-if="error" class="p-4 text-center text-sm text-muted-foreground">
            {{ error }}
          </p>
          <p v-else-if="records.length === 0" class="p-8 text-center text-sm text-muted-foreground">
            该标签下暂无记录
          </p>
          <template v-else>
            <button
              v-for="r in records"
              :key="r.id"
              type="button"
              class="flex w-full items-baseline justify-between gap-3 rounded-md px-3 py-2 text-left transition-colors hover:bg-accent"
              @click="emit('jump', r)"
            >
              <span class="min-w-0 flex-1 truncate text-sm">{{ r.title || "（无标题）" }}</span>
              <span class="shrink-0 text-xs text-muted-foreground">
                {{ fmtDateCn(r.record_date) }}
              </span>
              <span class="shrink-0 text-[11px] tabular-nums text-muted-foreground">
                {{ fmtTime(r.updated_at) }}
              </span>
            </button>
            <p
              v-if="records.length >= MAX_LIST"
              class="px-3 pt-2 text-center text-xs text-muted-foreground"
            >
              仅显示前 {{ MAX_LIST }} 条
            </p>
          </template>
        </div>
      </div>
    </div>
  </Teleport>
</template>
