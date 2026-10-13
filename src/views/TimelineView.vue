<script setup lang="ts">
/**
 * 时间线视图：置顶记录抽为「置顶」组固定最前（组内按日期倒序），
 * 其余按日期倒序分组展示，分页加载（每页 50 条）。
 * 后端排序已保证置顶记录集中在分页数据流最前，前端按 is_pinned 抽组即可。
 * 点击"编辑"跳转主页对应日期。
 */
import { computed, onMounted, ref } from "vue";
import { useRouter } from "vue-router";
import { Pencil, Pin } from "@lucide/vue";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { toast } from "@/components/ui/toast";
import * as api from "@/api/records";
import { errMsg, splitTags } from "@/lib/utils";
import { fmtDateCn, fmtTime } from "@/composables/useDate";
import type { WorkRecord } from "@/types/work-record";

const PAGE = 50;

const records = ref<WorkRecord[]>([]);
const loading = ref(false);
const finished = ref(false);
const router = useRouter();

/** 置顶组在前，其余按日期分组；置顶组跨日期，组内每条自带日期显示 */
const groups = computed(() => {
  type Group = { date: string; items: WorkRecord[]; pinned: boolean };
  const pinned: Group[] = [];
  const normal: Group[] = [];
  for (const r of records.value) {
    const bucket = r.is_pinned ? pinned : normal;
    const last = bucket[bucket.length - 1];
    if (last && last.date === r.record_date) last.items.push(r);
    else bucket.push({ date: r.record_date, items: [r], pinned: r.is_pinned });
  }
  return [...pinned, ...normal];
});

async function loadMore(): Promise<void> {
  if (loading.value || finished.value) return;
  loading.value = true;
  try {
    const batch = await api.timeline(records.value.length, PAGE);
    if (batch.length < PAGE) finished.value = true;
    records.value.push(...batch);
  } catch (e) {
    toast({ title: "加载时间线失败", description: errMsg(e), variant: "destructive" });
  } finally {
    loading.value = false;
  }
}

function goEdit(date: string): void {
  void router.push({ path: "/", query: { date } });
}

onMounted(() => {
  void loadMore();
});
</script>

<template>
  <div class="flex h-full flex-col">
    <header class="flex h-14 shrink-0 items-center justify-between border-b px-4">
      <h1 class="text-sm font-semibold">时间线</h1>
      <span class="text-xs text-muted-foreground">全部有效记录 · 按时间倒序</span>
    </header>

    <div class="min-h-0 flex-1 overflow-y-auto p-4">
      <div class="mx-auto max-w-3xl space-y-4">
        <template v-if="records.length > 0">
          <section
            v-for="g in groups"
            :key="g.pinned ? `pinned-${g.date}` : g.date"
            class="space-y-1.5"
          >
            <h2 class="flex items-center gap-2 pt-2 text-xs font-medium text-muted-foreground">
              <template v-if="g.pinned">
                <Pin class="h-3 w-3 text-primary" />
                置顶
              </template>
              <template v-else>
                {{ fmtDateCn(g.date) }}
              </template>
              <span class="text-[11px] opacity-70">{{ g.items.length }} 条</span>
            </h2>
            <div
              v-for="r in g.items"
              :key="r.id"
              class="group flex items-start gap-3 rounded-md border p-3 transition-colors hover:bg-accent/60"
            >
              <div class="min-w-0 flex-1">
                <div class="flex items-baseline justify-between gap-2">
                  <p class="flex min-w-0 items-center gap-1 truncate text-sm font-medium">
                    <Pin v-if="r.is_pinned" class="h-3.5 w-3.5 shrink-0 text-primary" />
                    <span class="truncate">{{ r.title || "（无标题）" }}</span>
                    <!-- 置顶组跨日期，显示日期徽标区分归属 -->
                    <span
                      v-if="g.pinned"
                      class="shrink-0 rounded bg-accent px-1 text-[10px] font-normal text-muted-foreground"
                    >
                      {{ fmtDateCn(r.record_date) }}
                    </span>
                  </p>
                  <span class="shrink-0 text-[11px] tabular-nums text-muted-foreground">
                    {{ fmtTime(r.updated_at) }}
                  </span>
                </div>
                <p v-if="r.content_text" class="mt-0.5 line-clamp-2 text-xs leading-relaxed text-muted-foreground">
                  {{ r.content_text }}
                </p>
                <div v-if="splitTags(r.tags).length > 0" class="mt-1.5 flex flex-wrap gap-1">
                  <Badge
                    v-for="t in splitTags(r.tags)"
                    :key="t"
                    variant="outline"
                    class="h-4 px-1.5 text-[10px]"
                  >
                    {{ t }}
                  </Badge>
                </div>
              </div>
              <Button
                variant="ghost"
                size="sm"
                class="shrink-0 gap-1 text-muted-foreground opacity-0 group-hover:opacity-100"
                @click="goEdit(r.record_date)"
              >
                <Pencil class="h-3.5 w-3.5" />编辑
              </Button>
            </div>
          </section>
        </template>

        <template v-else-if="loading">
          <div v-for="i in 5" :key="i" class="space-y-2 rounded-md border p-3">
            <Skeleton class="h-4 w-1/3" />
            <Skeleton class="h-3 w-full" />
          </div>
        </template>

        <p v-else class="py-16 text-center text-sm text-muted-foreground">
          还没有任何工作记录，去主页新建一条吧
        </p>

        <div v-if="!finished && records.length > 0" class="flex justify-center pb-4 pt-1">
          <Button variant="outline" size="sm" :disabled="loading" @click="loadMore">
            {{ loading ? "加载中…" : "加载更多" }}
          </Button>
        </div>
      </div>
    </div>
  </div>
</template>
