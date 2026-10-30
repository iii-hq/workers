import { motion } from 'motion/react'
import { type ReactNode, useEffect, useRef } from 'react'
import { cn } from '@/lib/utils'

/* Settings motion is deliberately quiet, in the Linear register: nothing in
   the sidebar animates, a page swap is a 120ms opacity crossfade, and only
   the save bar moves (8px) because it arrives from off the canvas. One
   ease-out curve and two durations cover everything. */
export const settingsEase = [0.2, 0, 0, 1] as const
export const settingsFade = { duration: 0.12, ease: settingsEase } as const
export const settingsEnter = { duration: 0.16, ease: settingsEase } as const

interface SettingsPageTransitionProps {
  /** Changing the key crossfades the next page in; the first mount is still. */
  pageKey: string
  className?: string
  children: ReactNode
}

/**
 * The settings content column. A page change is an occasional, user
 * initiated move; the new page fades in over 120ms with no displacement, so
 * it reads the same with and without reduced motion. Nothing animates on
 * first paint.
 */
export function SettingsPageTransition({
  pageKey,
  className,
  children,
}: SettingsPageTransitionProps) {
  const mounted = useRef(false)
  useEffect(() => {
    mounted.current = true
  }, [])
  return (
    <motion.div
      key={pageKey}
      initial={mounted.current ? { opacity: 0 } : false}
      animate={{ opacity: 1 }}
      transition={settingsFade}
      className={cn('flex min-h-0 min-w-0 flex-1 flex-col', className)}
    >
      {children}
    </motion.div>
  )
}
