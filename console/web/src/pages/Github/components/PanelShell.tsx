import { AlertCircle, type LucideIcon } from 'lucide-react'
import type { ReactNode } from 'react'
import { EmptyState } from '@/components/ui/EmptyState'
import { StatusPanel } from '@/components/ui/StatusPanel'

interface PanelShellProps {
  loading: boolean
  error: string | null
  empty: boolean
  emptyIcon: LucideIcon
  emptyTitle: string
  emptyDescription: string
  children: ReactNode
}

/**
 * Shared error / first-load / empty scaffolding for the Github panels.
 * Worker errors carry gh's own stderr (auth failures, 404s), so the alert
 * detail is already the actionable message.
 */
export function PanelShell({
  loading,
  error,
  empty,
  emptyIcon,
  emptyTitle,
  emptyDescription,
  children,
}: PanelShellProps) {
  if (error) {
    return (
      <StatusPanel
        variant="alert"
        icon={<AlertCircle className="w-full h-full" />}
        headline="github call failed"
        detail={error}
      />
    )
  }
  if (loading && empty) {
    return (
      <p className="font-mono text-[12px] lowercase text-ink-ghost">
        loading...
      </p>
    )
  }
  if (empty) {
    return (
      <EmptyState
        icon={emptyIcon}
        title={emptyTitle}
        description={emptyDescription}
      />
    )
  }
  return <>{children}</>
}
