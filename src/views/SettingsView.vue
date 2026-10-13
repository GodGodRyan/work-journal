<script setup lang="ts">
/**
 * 设置页面：外观（主题）/ 数据管理（备份·恢复·数据库位置）/ 快捷键说明 / 关于。
 * 恢复备份前有醒目警告；恢复成功后全量刷新所有页面数据。
 */
import { onMounted, ref } from "vue";
import { Database, Info, Keyboard, Palette, RotateCcw, Download } from "@lucide/vue";
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
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { toast } from "@/components/ui/toast";
import * as api from "@/api/records";
import { modLabel } from "@/composables/useHotkeys";
import { useAppStore } from "@/stores/app";
import { errMsg } from "@/lib/utils";
import type { ThemeMode } from "@/types/work-record";

const store = useAppStore();

const dbPath = ref("");
const version = ref("");
const backingUp = ref(false);
const restoring = ref(false);

const themeOptions: { value: ThemeMode; label: string }[] = [
  { value: "light", label: "浅色" },
  { value: "dark", label: "深色" },
  { value: "system", label: "跟随系统" },
];

/** 主题切换：走 store.setTheme（写 localStorage + 立即应用），
 *  直接 v-model 写 state 会跳过持久化与 applyTheme。
 *  reka Select 的 AcceptableValue 含 number/boolean/null，仅接受 string 并忽略其余 */
function onThemeChange(v: unknown): void {
  if (typeof v === "string") store.setTheme(v as ThemeMode);
}

onMounted(async () => {
  try {
    [dbPath.value, version.value] = await Promise.all([api.dbPath(), api.appVersion()]);
  } catch (e) {
    toast({ title: "读取信息失败", description: errMsg(e), variant: "destructive" });
  }
});

async function backup(): Promise<void> {
  backingUp.value = true;
  try {
    const path = await api.backupDb();
    if (path) toast({ title: "备份成功", description: path });
  } catch (e) {
    toast({ title: "备份失败", description: errMsg(e), variant: "destructive" });
  } finally {
    backingUp.value = false;
  }
}

async function restore(): Promise<void> {
  restoring.value = true;
  try {
    const count = await api.restoreDb();
    if (count == null) return; // 用户取消
    await store.reloadAll();
    toast({
      title: "恢复成功",
      description: `已从备份恢复 ${count} 条有效记录，数据已全量刷新`,
    });
  } catch (e) {
    toast({ title: "恢复失败", description: errMsg(e), variant: "destructive" });
  } finally {
    restoring.value = false;
  }
}
</script>

<template>
  <div class="flex h-full flex-col">
    <header class="flex h-14 shrink-0 items-center border-b px-4">
      <h1 class="text-sm font-semibold">设置</h1>
    </header>

    <div class="min-h-0 flex-1 overflow-y-auto p-4">
      <div class="mx-auto max-w-2xl space-y-4">
        <!-- 外观 -->
        <Card>
          <CardHeader class="pb-3">
            <CardTitle class="flex items-center gap-2 text-sm">
              <Palette class="h-4 w-4 text-muted-foreground" />外观
            </CardTitle>
            <CardDescription>主题模式：浅色 / 深色 / 跟随系统</CardDescription>
          </CardHeader>
          <CardContent>
            <Select :model-value="store.theme" @update:model-value="onThemeChange">
              <SelectTrigger class="w-[220px]">
                <SelectValue placeholder="选择主题" />
              </SelectTrigger>
              <SelectContent>
                <SelectItem v-for="o in themeOptions" :key="o.value" :value="o.value">
                  {{ o.label }}
                </SelectItem>
              </SelectContent>
            </Select>
          </CardContent>
        </Card>

        <!-- 数据管理 -->
        <Card>
          <CardHeader class="pb-3">
            <CardTitle class="flex items-center gap-2 text-sm">
              <Database class="h-4 w-4 text-muted-foreground" />数据管理
            </CardTitle>
            <CardDescription>
              全部数据存储在本地 SQLite 数据库，无网络、无云端。恢复备份前会自动把当前数据备份到应用数据目录。
            </CardDescription>
          </CardHeader>
          <CardContent class="space-y-3">
            <div>
              <p class="text-xs text-muted-foreground">数据库位置</p>
              <code class="mt-1 block break-all rounded-md bg-muted px-2 py-1.5 text-xs">{{ dbPath || "…" }}</code>
            </div>
            <div class="flex items-center gap-2">
              <Button variant="outline" size="sm" class="gap-1" :disabled="backingUp" @click="backup">
                <Download class="h-3.5 w-3.5" />{{ backingUp ? "备份中…" : "备份数据库" }}
              </Button>
              <AlertDialog>
                <AlertDialogTrigger as-child>
                  <Button variant="outline" size="sm" class="gap-1 text-destructive hover:text-destructive" :disabled="restoring">
                    <RotateCcw class="h-3.5 w-3.5" />从备份恢复
                  </Button>
                </AlertDialogTrigger>
                <AlertDialogContent>
                  <AlertDialogHeader>
                    <AlertDialogTitle>从备份恢复数据？</AlertDialogTitle>
                    <AlertDialogDescription>
                      恢复将用所选备份文件覆盖当前全部数据。恢复前会自动生成一份当前数据的安全备份，
                      但仍建议先手动备份一次。
                    </AlertDialogDescription>
                  </AlertDialogHeader>
                  <AlertDialogFooter>
                    <AlertDialogCancel>取消</AlertDialogCancel>
                    <AlertDialogAction :disabled="restoring" @click="restore">
                      {{ restoring ? "恢复中…" : "选择备份文件并恢复" }}
                    </AlertDialogAction>
                  </AlertDialogFooter>
                </AlertDialogContent>
              </AlertDialog>
            </div>
          </CardContent>
        </Card>

        <!-- 快捷键 -->
        <Card>
          <CardHeader class="pb-3">
            <CardTitle class="flex items-center gap-2 text-sm">
              <Keyboard class="h-4 w-4 text-muted-foreground" />快捷键
            </CardTitle>
          </CardHeader>
          <CardContent>
            <ul class="space-y-1.5 text-sm">
              <li class="flex items-center justify-between">
                <span class="text-muted-foreground">新建工作记录</span>
                <kbd class="rounded border bg-muted px-1.5 py-0.5 text-xs">{{ modLabel }}+N</kbd>
              </li>
              <li class="flex items-center justify-between">
                <span class="text-muted-foreground">聚焦搜索输入框</span>
                <kbd class="rounded border bg-muted px-1.5 py-0.5 text-xs">{{ modLabel }}+F</kbd>
              </li>
              <li class="flex items-center justify-between">
                <span class="text-muted-foreground">手动保存（自动保存兜底）</span>
                <kbd class="rounded border bg-muted px-1.5 py-0.5 text-xs">{{ modLabel }}+S</kbd>
              </li>
              <li class="flex items-center justify-between">
                <span class="text-muted-foreground">打开设置</span>
                <kbd class="rounded border bg-muted px-1.5 py-0.5 text-xs">{{ modLabel }}+,</kbd>
              </li>
            </ul>
          </CardContent>
        </Card>

        <!-- 关于 -->
        <Card>
          <CardHeader class="pb-3">
            <CardTitle class="flex items-center gap-2 text-sm">
              <Info class="h-4 w-4 text-muted-foreground" />关于
            </CardTitle>
          </CardHeader>
          <CardContent class="space-y-1 text-sm text-muted-foreground">
            <p>工作日志 · 每日工作记录记事本</p>
            <p>版本 {{ version || "…" }}</p>
            <p class="text-xs">Tauri 2 · Vue 3 + TypeScript · SQLite (FTS5) · shadcn-vue + Tailwind CSS</p>
            <p class="text-xs">数据完全本地存储，无网络请求、无登录、无云端同步</p>
          </CardContent>
        </Card>
      </div>
    </div>
  </div>
</template>
