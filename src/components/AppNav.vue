<script setup lang="ts">
/**
 * 左侧导航栏：页面切换 + 顶部折叠/展开。
 * 折叠状态存 localStorage（UI 偏好，非业务数据），首次进入默认展开；
 * 折叠后只显示图标，悬停有名称提示。
 */
import { ref } from "vue";
import {
  History,
  Home,
  PanelLeftClose,
  PanelLeftOpen,
  Search,
  Settings,
  Trash2,
} from "@lucide/vue";
import logoImg from "@/assets/logo.png";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";

const NAV_COLLAPSED_KEY = "work-journal-nav-collapsed";

/** 折叠状态（"1" 为折叠；无记录/其他值一律视为展开，首次进入即默认展开） */
const collapsed = ref(localStorage.getItem(NAV_COLLAPSED_KEY) === "1");

function toggleCollapsed(): void {
  collapsed.value = !collapsed.value;
  localStorage.setItem(NAV_COLLAPSED_KEY, collapsed.value ? "1" : "0");
}

const navItems = [
  { to: "/", label: "主页", icon: Home },
  { to: "/timeline", label: "时间线", icon: History },
  { to: "/search", label: "搜索", icon: Search },
  { to: "/trash", label: "回收站", icon: Trash2 },
  { to: "/settings", label: "设置", icon: Settings },
] as const;
</script>

<template>
  <aside
    :class="
      cn(
        'flex shrink-0 flex-col border-r bg-card transition-[width] duration-200',
        collapsed ? 'w-14' : 'w-52',
      )
    "
  >
    <!-- Logo + 顶部折叠按钮（折叠态：logo + 按钮；展开态：logo + 文字 + 按钮） -->
    <div
      class="flex h-14 shrink-0 items-center border-b"
      :class="collapsed ? 'justify-center gap-1 px-1' : 'gap-2 px-2'"
    >
      <!-- PNG 自带透明圆角（四角 alpha=0），无需 CSS 圆角裁剪 -->
      <img
        :src="logoImg"
        alt="工作日志"
        class="shrink-0"
        :class="collapsed ? 'h-6 w-6' : 'h-7 w-7'"
      />
      <span v-if="!collapsed" class="flex-1 truncate text-sm font-semibold">工作日志</span>
      <TooltipProvider :delay-duration="0">
        <Tooltip>
          <TooltipTrigger as-child>
            <Button
              variant="ghost"
              size="icon"
              class="shrink-0"
              :class="collapsed ? 'h-6 w-6' : 'h-7 w-7'"
              :aria-label="collapsed ? '展开侧栏' : '折叠侧栏'"
              @click="toggleCollapsed"
            >
              <component
                :is="collapsed ? PanelLeftOpen : PanelLeftClose"
                class="h-4 w-4"
              />
            </Button>
          </TooltipTrigger>
          <TooltipContent side="right">
            <p>{{ collapsed ? "展开侧栏" : "折叠侧栏" }}</p>
          </TooltipContent>
        </Tooltip>
      </TooltipProvider>
    </div>

    <!-- 页面导航 -->
    <nav class="flex-1 space-y-1 overflow-y-auto p-2">
      <RouterLink
        v-for="item in navItems"
        :key="item.to"
        :to="item.to"
        custom
        v-slot="{ href, navigate, isActive }"
      >
        <a
          :href="href"
          :title="collapsed ? item.label : undefined"
          :class="cn(
            'flex items-center rounded-md text-sm transition-colors',
            collapsed ? 'justify-center px-0 py-2' : 'gap-3 px-3 py-2',
            isActive
              ? 'bg-accent font-medium text-accent-foreground'
              : 'text-muted-foreground hover:bg-accent/60 hover:text-foreground',
          )"
          @click="navigate"
        >
          <component :is="item.icon" class="h-4 w-4 shrink-0" />
          <span v-if="!collapsed">{{ item.label }}</span>
        </a>
      </RouterLink>
    </nav>
  </aside>
</template>
