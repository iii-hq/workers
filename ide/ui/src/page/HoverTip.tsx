import { Tooltip, TooltipContent, TooltipTrigger } from '@iii-dev/console-ui'
import type { ReactElement } from 'react'

export function HoverTip({
  label,
  children,
}: {
  label: string
  children: ReactElement
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>{children}</TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}
