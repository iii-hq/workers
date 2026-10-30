import { Columns2, PanelLeft, PanelRight, Square } from 'lucide-react'
import { type RefObject, useLayoutEffect, useRef, useState } from 'react'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from '@/components/shadcn/dropdown-menu'
import { IconButton } from '@/components/ui/IconButton'
import { formatBinding, shortcutPlatform } from '@/lib/keybindings/bindings'
import {
  bindingsFor,
  type KeybindingActionId,
} from '@/lib/keybindings/registry'
import { usePaneActions } from '@/lib/pane-actions'
import { useScreenOptions } from './use-screen-options'

/** Below this header width the control steps aside: the page's own title
    and read-outs need the room more (the keys still split the pane). */
const MIN_HEADER_WIDTH = 480

/** Whether the header around `ref` has room for one more control. */
function useHeaderRoom(
  ref: RefObject<HTMLElement | null>,
  mounted: boolean,
): boolean {
  const [roomy, setRoomy] = useState(true)
  useLayoutEffect(() => {
    if (!mounted) return
    const header = ref.current?.closest('header')
    if (!header || typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(([entry]) =>
      setRoomy(entry.contentRect.width >= MIN_HEADER_WIDTH),
    )
    observer.observe(header)
    return () => observer.disconnect()
  }, [ref, mounted])
  return roomy
}

/** A menu row's chord: `⌃]` on a Mac, `alt+]` elsewhere. */
function shortcutText(id: KeybindingActionId): string | null {
  const platform = shortcutPlatform()
  const binding = bindingsFor(id, platform)[0]
  return binding
    ? formatBinding(binding, platform).join(platform === 'mac' ? '' : '+')
    : null
}

/**
 * The pane's split control, at the end of its header: what to open beside
 * this pane (it lands on the right, at half this pane's width), an empty
 * pane on either side (the ⌃[ / ⌃] keys do the same for the pane holding
 * focus), and the way back to one pane. Renders nothing outside a workspace
 * pane.
 */
export function PaneSplitMenu({ className }: { className?: string }) {
  const actions = usePaneActions()
  const { screenOptions } = useScreenOptions()
  const slotRef = useRef<HTMLSpanElement>(null)
  const roomy = useHeaderRoom(slotRef, actions !== undefined)
  if (!actions) return null
  const open = new Set(actions.openScreens)
  const beside = screenOptions.filter((option) => !open.has(option.value))
  const splitRight = shortcutText('panel.split')
  const splitLeft = shortcutText('panel.splitLeft')
  return (
    <span ref={slotRef} className={roomy ? 'contents' : 'hidden'}>
      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <IconButton
              label="Split pane"
              tooltipSide="bottom"
              className={className}
              data-pane-split-trigger=""
            >
              <Columns2 aria-hidden className="size-4 shrink-0" />
            </IconButton>
          }
        />
        <DropdownMenuContent align="end" className="min-w-[14rem]">
          {beside.length > 0 ? (
            <>
              <DropdownMenuGroup>
                <DropdownMenuLabel>Open to the right</DropdownMenuLabel>
                {beside.map((option) => {
                  const Icon = option.icon
                  return (
                    <DropdownMenuItem
                      key={option.value}
                      disabled={actions.busy}
                      onClick={() => actions.split('right', option.value)}
                    >
                      <Icon aria-hidden />
                      {option.label}
                    </DropdownMenuItem>
                  )
                })}
              </DropdownMenuGroup>
              <DropdownMenuSeparator />
            </>
          ) : null}
          <DropdownMenuItem
            disabled={actions.busy}
            onClick={() => actions.split('right')}
          >
            <PanelRight aria-hidden />
            Split right
            {splitRight ? (
              <DropdownMenuShortcut>{splitRight}</DropdownMenuShortcut>
            ) : null}
          </DropdownMenuItem>
          <DropdownMenuItem
            disabled={actions.busy}
            onClick={() => actions.split('left')}
          >
            <PanelLeft aria-hidden />
            Split left
            {splitLeft ? (
              <DropdownMenuShortcut>{splitLeft}</DropdownMenuShortcut>
            ) : null}
          </DropdownMenuItem>
          {actions.closeOthers ? (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                disabled={actions.busy}
                onClick={actions.closeOthers}
              >
                <Square aria-hidden />
                Close other panes
              </DropdownMenuItem>
            </>
          ) : null}
        </DropdownMenuContent>
      </DropdownMenu>
    </span>
  )
}
