<script lang="ts" setup>
import type { RangeCalendarCellProps } from "reka-ui"
import type { HTMLAttributes } from "vue"
import { reactiveOmit } from "@vueuse/core"
import { RangeCalendarCell, useForwardProps } from "reka-ui"
import { cn } from "@/lib/utils"

const props = defineProps<RangeCalendarCellProps & { class?: HTMLAttributes["class"] }>()

const delegatedProps = reactiveOmit(props, "class")

const forwardedProps = useForwardProps(delegatedProps)
</script>

<template>
  <RangeCalendarCell
    :class="cn(
      'relative h-7 w-7 p-0 text-center text-sm focus-within:relative focus-within:z-20 [&:has([data-selection-end])]:rounded-r-md [&:has([data-selection-start])]:rounded-l-md [&:has([data-highlighted])]:bg-accent [&:has([data-highlighted])]:text-accent-foreground [&:has([data-highlighted])]:rounded-none',
      props.class,
    )"
    v-bind="forwardedProps"
  >
    <slot />
  </RangeCalendarCell>
</template>
