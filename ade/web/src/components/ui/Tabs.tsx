import uiClasses from '@iii-dev/console-ui/ui-classes'
import * as TabsPrimitive from '@radix-ui/react-tabs'
import * as React from 'react'
import { cn } from '@/lib/utils'
import { TabIconSlot } from './TabIcon'

export const Tabs = TabsPrimitive.Root

export interface TabsListProps
  extends React.ComponentPropsWithoutRef<typeof TabsPrimitive.List> {
  /** `line` is the Console content-navigation default. */
  variant?: 'line'
}

export const TabsList = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.List>,
  TabsListProps
>(({ className, variant = 'line', ...props }, ref) => (
  <TabsPrimitive.List
    ref={ref}
    data-variant={variant}
    className={cn(uiClasses.tabsList, className)}
    {...props}
  />
))
TabsList.displayName = 'TabsList'

export interface TabsTriggerProps
  extends React.ComponentPropsWithoutRef<typeof TabsPrimitive.Trigger> {
  /** Defaults to a semantic 16px icon inferred from `value`; `false` hides it. */
  icon?: React.ReactNode | false
}

export const TabsTrigger = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.Trigger>,
  TabsTriggerProps
>(({ className, icon, value, children, ...props }, ref) => {
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
      if (node.getAttribute('data-state') !== 'active') return
      node.scrollIntoView({ block: 'nearest', inline: 'nearest' })
    }
    sync()
    const observer = new MutationObserver(sync)
    observer.observe(node, { attributes: true, attributeFilter: ['data-state'] })
    return () => observer.disconnect()
  }, [])

  return (
    <TabsPrimitive.Trigger
      ref={setRefs}
      value={value}
      className={cn(uiClasses.tab, className)}
      {...props}
    >
      <TabIconSlot icon={icon} value={value} />
      <span className="min-w-0 truncate">{children}</span>
    </TabsPrimitive.Trigger>
  )
})
TabsTrigger.displayName = 'TabsTrigger'

export const TabsContent = React.forwardRef<
  React.ElementRef<typeof TabsPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Content>
>(({ className, ...props }, ref) => (
  <TabsPrimitive.Content
    ref={ref}
    className={cn('focus-visible:outline-none', className)}
    {...props}
  />
))
TabsContent.displayName = 'TabsContent'
