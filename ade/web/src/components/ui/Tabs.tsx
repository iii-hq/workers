/**
 * The console's content-navigation tabs: the shadcn (Base UI) tabs with the
 * `line` look by default, a semantic 16px icon per trigger (`TabIconSlot`)
 * and the active tab kept in view when the list overflows. Worker UIs get
 * the same parts through `@iii-dev/console-ui`.
 */

import uiClasses from '@iii-dev/console-ui/ui-classes'
import * as React from 'react'
import {
  Tabs as BaseTabs,
  TabsContent as BaseTabsContent,
  TabsList as BaseTabsList,
  TabsTrigger as BaseTabsTrigger,
} from '@/components/shadcn/tabs'
import { cn } from '@/lib/utils'
import { TabIconSlot } from './TabIcon'

export const Tabs = BaseTabs

export interface TabsListProps
  extends React.ComponentProps<typeof BaseTabsList> {
  /** `line` is the Console content-navigation default. */
  variant?: 'line' | 'default'
}

export function TabsList({
  className,
  variant = 'line',
  ...props
}: TabsListProps) {
  return (
    <BaseTabsList
      variant={variant}
      className={cn(variant === 'line' && uiClasses.tabsList, className)}
      {...props}
    />
  )
}

export interface TabsTriggerProps
  extends React.ComponentProps<typeof BaseTabsTrigger> {
  /** Defaults to a semantic 16px icon inferred from `value`; `false` hides it. */
  icon?: React.ReactNode | false
}

export function TabsTrigger({
  className,
  icon,
  value,
  children,
  ref,
  ...props
}: TabsTriggerProps) {
  const innerRef = React.useRef<HTMLButtonElement | null>(null)
  const setRefs = React.useCallback(
    (node: HTMLButtonElement | null) => {
      innerRef.current = node
      if (typeof ref === 'function') ref(node)
      else if (ref) ref.current = node
    },
    [ref],
  )

  // Keep the active tab in view when the list overflows horizontally
  // (narrow panes / mobile). Only scrolls the tab list, never the page.
  React.useEffect(() => {
    const node = innerRef.current
    if (!node) return
    const list = node.parentElement
    if (!list || list.scrollWidth <= list.clientWidth) return
    const sync = () => {
      if (!node.hasAttribute('data-active')) return
      node.scrollIntoView({ block: 'nearest', inline: 'nearest' })
    }
    sync()
    const observer = new MutationObserver(sync)
    observer.observe(node, {
      attributes: true,
      attributeFilter: ['data-active'],
    })
    return () => observer.disconnect()
  }, [])

  return (
    <BaseTabsTrigger
      ref={setRefs}
      value={value}
      className={cn(uiClasses.tab, className)}
      {...props}
    >
      <TabIconSlot icon={icon} value={value} />
      <span className="min-w-0 truncate">{children}</span>
    </BaseTabsTrigger>
  )
}

export const TabsContent = BaseTabsContent
