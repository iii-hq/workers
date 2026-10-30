import type * as React from 'react'
import {
  Card,
  CardContent,
  CardHeader,
  CardTitle,
} from '@/components/shadcn/card'

interface CellProps {
  title?: React.ReactNode
  children: React.ReactNode
  className?: string
}

/** A titled note on the canvas, set as a shadcn Card: Inter 14px, the copy in
 *  faint ink under a medium title. */
export function Cell({ title, children, className }: CellProps) {
  return (
    <Card className={className}>
      {title ? (
        <CardHeader>
          <CardTitle className="text-ink">{title}</CardTitle>
        </CardHeader>
      ) : null}
      <CardContent className="max-w-[34ch] leading-relaxed text-ink-faint">
        {children}
      </CardContent>
    </Card>
  )
}
