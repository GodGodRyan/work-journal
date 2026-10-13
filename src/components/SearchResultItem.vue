<script setup lang="ts">
/**
 * 搜索结果项：标题/内容关键词高亮（前端按命中词分段，纯文本渲染 + <mark>，无 v-html 注入面），
 * 点击铅笔跳转主页对应日期编辑。
 */
import { useRouter } from "vue-router";
import { Pencil } from "@lucide/vue";
import { Badge } from "@/components/ui/badge";
import { maskAttachmentMarkers, snippetAroundTerm, splitByTerms, splitTags } from "@/lib/utils";
import { fmtDateCn, fmtTime } from "@/composables/useDate";
import type { WorkRecord } from "@/types/work-record";

const props = defineProps<{
  record: WorkRecord;
  /** 命中词列表（来自搜索框，按空白拆分） */
  terms: string[];
}>();
const router = useRouter();

function goEdit(): void {
  void router.push({ path: "/", query: { date: props.record.record_date } });
}
</script>

<template>
  <div class="group rounded-md border p-3 transition-colors hover:bg-accent/60">
    <div class="flex items-start justify-between gap-2">
      <div class="min-w-0 flex-1">
        <p class="truncate text-sm font-medium">
          <template
            v-for="(seg, i) in splitByTerms(props.record.title, props.terms)"
            :key="`t${i}`"
          >
            <mark v-if="seg.hit" class="rounded-sm bg-primary/20 px-0.5">{{ seg.text }}</mark>
            <template v-else>{{ seg.text }}</template>
          </template>
        </p>
        <p
          v-if="props.record.content_text"
          class="mt-1 line-clamp-2 text-xs leading-relaxed text-muted-foreground"
        >
          <template
            v-for="(seg, i) in splitByTerms(snippetAroundTerm(maskAttachmentMarkers(props.record.content_text), props.terms), props.terms)"
            :key="`c${i}`"
          >
            <mark v-if="seg.hit" class="rounded-sm bg-primary/20 px-0.5">{{ seg.text }}</mark>
            <template v-else>{{ seg.text }}</template>
          </template>
        </p>
        <div v-if="splitTags(props.record.tags).length > 0" class="mt-1.5 flex flex-wrap items-center gap-1.5">
          <Badge
            v-for="t in splitTags(props.record.tags)"
            :key="t"
            variant="outline"
            class="h-4 px-1.5 text-[10px]"
          >
            {{ t }}
          </Badge>
        </div>
      </div>
      <div class="flex shrink-0 flex-col items-end gap-1">
        <button
          type="button"
          class="rounded-md p-1 text-muted-foreground opacity-0 transition-opacity hover:bg-accent hover:text-foreground group-hover:opacity-100"
          title="去主页编辑该记录"
          @click="goEdit"
        >
          <Pencil class="h-3.5 w-3.5" />
        </button>
        <span class="text-[11px] tabular-nums text-muted-foreground">
          {{ fmtDateCn(props.record.record_date) }}
        </span>
        <span class="text-[11px] tabular-nums text-muted-foreground">
          {{ fmtTime(props.record.updated_at) }}
        </span>
      </div>
    </div>
  </div>
</template>
