<script setup lang="ts">
/**
 * 回收站：软删除记录列表，支持恢复 / 彻底删除（确认）/ 清空（确认）。
 */
import { onMounted, ref } from "vue";
import { RotateCcw, Trash2 } from "@lucide/vue";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { toast } from "@/components/ui/toast";
import * as api from "@/api/records";
import { useAppStore } from "@/stores/app";
import { errMsg, splitTags } from "@/lib/utils";
import { fmtDateCn, fmtTime } from "@/composables/useDate";
import type { WorkRecord } from "@/types/work-record";

const store = useAppStore();
const records = ref<WorkRecord[]>([]);
const loading = ref(false);
const busyId = ref<number | null>(null);

async function load(): Promise<void> {
  loading.value = true;
  try {
    records.value = await api.trashRecords();
  } catch (e) {
    toast({ title: "加载回收站失败", description: errMsg(e), variant: "destructive" });
  } finally {
    loading.value = false;
  }
}

async function restore(id: number): Promise<void> {
  busyId.value = id;
  try {
    await api.restoreRecord(id);
    await store.refreshAll();
    await load();
    toast({ title: "已恢复", description: "记录已回到原日期" });
  } catch (e) {
    toast({ title: "恢复失败", description: errMsg(e), variant: "destructive" });
  } finally {
    busyId.value = null;
  }
}

async function purge(id: number): Promise<void> {
  busyId.value = id;
  try {
    await api.purgeRecord(id);
    await load();
    toast({ title: "已彻底删除", description: "该记录无法再恢复" });
  } catch (e) {
    toast({ title: "删除失败", description: errMsg(e), variant: "destructive" });
  } finally {
    busyId.value = null;
  }
}

async function emptyAll(): Promise<void> {
  try {
    const n = await api.emptyTrash();
    await load();
    toast({ title: "回收站已清空", description: `共删除 ${n} 条记录` });
  } catch (e) {
    toast({ title: "清空失败", description: errMsg(e), variant: "destructive" });
  }
}

onMounted(() => {
  void load();
});
</script>

<template>
  <div class="flex h-full flex-col">
    <header class="flex h-14 shrink-0 items-center justify-between border-b px-4">
      <h1 class="text-sm font-semibold">回收站</h1>
      <AlertDialog>
        <AlertDialogTrigger as-child>
          <Button variant="destructive" size="sm" :disabled="records.length === 0">
            <Trash2 class="h-3.5 w-3.5" />清空回收站
          </Button>
        </AlertDialogTrigger>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>清空回收站？</AlertDialogTitle>
            <AlertDialogDescription>
              将物理删除回收站中的全部 {{ records.length }} 条记录，此操作不可恢复。
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>取消</AlertDialogCancel>
            <AlertDialogAction @click="emptyAll">清空</AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </header>

    <div class="min-h-0 flex-1 overflow-y-auto p-4">
      <div class="mx-auto max-w-3xl space-y-1.5">
        <template v-if="records.length > 0">
          <div
            v-for="r in records"
            :key="r.id"
            class="flex items-start gap-3 rounded-md border p-3"
          >
            <div class="min-w-0 flex-1">
              <div class="flex items-baseline justify-between gap-2">
                <p class="truncate text-sm font-medium">{{ r.title || "（无标题）" }}</p>
                <span class="shrink-0 text-[11px] tabular-nums text-muted-foreground">
                  {{ fmtDateCn(r.record_date) }}
                </span>
              </div>
              <p v-if="r.content_text" class="mt-0.5 line-clamp-2 text-xs text-muted-foreground">
                {{ r.content_text }}
              </p>
              <div class="mt-1.5 flex flex-wrap items-center gap-1.5">
                <Badge
                  v-for="t in splitTags(r.tags).slice(0, 3)"
                  :key="t"
                  variant="outline"
                  class="h-4 px-1.5 text-[10px]"
                >
                  {{ t }}
                </Badge>
                <span class="text-[11px] text-muted-foreground">删除于 {{ fmtTime(r.updated_at) }}</span>
              </div>
            </div>
            <div class="flex shrink-0 items-center gap-1">
              <Button
                variant="outline"
                size="sm"
                class="gap-1"
                :disabled="busyId === r.id"
                @click="restore(r.id)"
              >
                <RotateCcw class="h-3.5 w-3.5" />恢复
              </Button>
              <AlertDialog>
                <AlertDialogTrigger as-child>
                  <Button
                    variant="ghost"
                    size="sm"
                    class="gap-1 text-muted-foreground hover:text-destructive"
                    :disabled="busyId === r.id"
                  >
                    <Trash2 class="h-3.5 w-3.5" />彻底删除
                  </Button>
                </AlertDialogTrigger>
                <AlertDialogContent>
                  <AlertDialogHeader>
                    <AlertDialogTitle>彻底删除？</AlertDialogTitle>
                    <AlertDialogDescription>
                      物理删除后无法恢复，建议先确认不再需要该记录。
                    </AlertDialogDescription>
                  </AlertDialogHeader>
                  <AlertDialogFooter>
                    <AlertDialogCancel>取消</AlertDialogCancel>
                    <AlertDialogAction @click="purge(r.id)">删除</AlertDialogAction>
                  </AlertDialogFooter>
                </AlertDialogContent>
              </AlertDialog>
            </div>
          </div>
        </template>
        <p v-else-if="loading" class="py-12 text-center text-sm text-muted-foreground">加载中…</p>
        <p v-else class="py-12 text-center text-sm text-muted-foreground">
          回收站是空的
        </p>
      </div>
    </div>
  </div>
</template>
