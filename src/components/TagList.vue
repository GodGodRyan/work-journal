<script setup lang="ts">
/**
 * 标签列表：展示全部已使用标签及计数；点击标签打开标签弹窗视图（emit selectTag）。
 */
import { Tag } from "@lucide/vue";
import { Badge } from "@/components/ui/badge";
import { useAppStore } from "@/stores/app";

const store = useAppStore();

const emit = defineEmits<{
  selectTag: [tag: string];
}>();
</script>

<template>
  <div class="space-y-1.5">
    <h3 class="text-xs font-medium text-muted-foreground">标签</h3>
    <p v-if="store.tags.length === 0" class="text-[11px] text-muted-foreground">
      暂无标签，编辑记录时输入回车即可添加
    </p>
    <!-- 限高滚动：标签多时不挤压上方置顶区与日历 -->
    <div v-else class="max-h-24 flex flex-wrap gap-1.5 overflow-y-auto pr-0.5">
      <Badge
        v-for="t in store.tags"
        :key="t.tag"
        variant="outline"
        class="cursor-pointer select-none gap-1"
        @click="emit('selectTag', t.tag)"
      >
        <Tag class="h-3 w-3" />
        {{ t.tag }}
        <span class="opacity-70">{{ t.count }}</span>
      </Badge>
    </div>
  </div>
</template>
