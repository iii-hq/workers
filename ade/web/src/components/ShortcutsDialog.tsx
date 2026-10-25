/**
 * The keyboard-shortcuts reference, generated from the registry so it
 * cannot drift from what the keys actually do. Each chord is spelled for
 * the reader's keyboard and set in key caps; the groups are the registry's
 * own, ordered from "anywhere" to "inside the palette".
 */

import { Keyboard } from 'lucide-react'
import { Fragment } from 'react'
import { Button } from '@/components/shadcn/button'
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/shadcn/dialog'
import { KeyCombo } from '@/components/ui/KeyCombo'
import { type Platform, shortcutPlatform } from '@/lib/keybindings/bindings'
import {
  bindingsFor,
  type KeybindingDefinition,
  keybindingGroups,
  resolveBindings,
} from '@/lib/keybindings/registry'

/** Reading order, and a line under each heading saying where the keys
 *  work. A group the registry adds later lands at the end, undescribed. */
const GROUPS: ReadonlyArray<{ group: string; detail: string }> = [
  { group: 'ADE', detail: 'From anywhere in the console' },
  { group: 'Chat', detail: 'Start work, from anywhere' },
  { group: 'Go to', detail: 'Jump straight to a page' },
  { group: 'Workspace', detail: 'Tabs and the panels inside them' },
  { group: 'Search', detail: 'While the ⌘K palette is open' },
]

/** A key cap: a flat hairline box, no shadow, so it reads as a label. */
const KEYCAP =
  'h-6 min-w-6 rounded-md border border-border bg-background px-1.5 font-inter text-[11px] font-medium text-foreground shadow-none'

export function orderedShortcutGroups(
  platform: Platform,
): Array<{ group: string; detail: string; entries: KeybindingDefinition[] }> {
  const byGroup = new Map(
    keybindingGroups().map(([group, entries]) => [
      group,
      entries.filter(
        (entry) => resolveBindings(entry.bindings, platform).length > 0,
      ),
    ]),
  )
  const ordered = GROUPS.filter(({ group }) => byGroup.has(group)).map(
    ({ group, detail }) => ({
      group,
      detail,
      entries: byGroup.get(group) ?? [],
    }),
  )
  for (const [group, entries] of byGroup) {
    if (GROUPS.some((known) => known.group === group)) continue
    ordered.push({ group, detail: '', entries })
  }
  return ordered.filter(({ entries }) => entries.length > 0)
}

export interface ShortcutsDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
}

export function ShortcutsDialog({ open, onOpenChange }: ShortcutsDialogProps) {
  const platform = shortcutPlatform()
  const groups = orderedShortcutGroups(platform)
  const own = bindingsFor('app.shortcuts', platform)[0]
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[min(46rem,calc(100dvh-2rem))] flex-col gap-0 p-0 [--dialog-max-width:46rem]">
        <DialogHeader className="flex-row items-start gap-3 border-b border-border px-5 py-4 pr-14">
          <span
            aria-hidden
            className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-muted text-foreground"
          >
            <Keyboard className="size-4" />
          </span>
          <div className="flex min-w-0 flex-col gap-1">
            <DialogTitle>Keyboard shortcuts</DialogTitle>
            <DialogDescription>
              Every shortcut in the ADE, spelled for your keyboard.
              {own ? (
                <>
                  {' '}
                  Press{' '}
                  <KeyCombo
                    binding={own}
                    platform={platform}
                    className="align-text-bottom"
                    capClassName={KEYCAP}
                  />{' '}
                  anywhere to open this list.
                </>
              ) : null}
            </DialogDescription>
          </div>
        </DialogHeader>

        {/* Focusable so the arrow keys scroll it when the list is taller
            than the dialog. */}
        <div
          // biome-ignore lint/a11y/noNoninteractiveTabindex: the scrolling region must be reachable by keyboard
          tabIndex={0}
          className="@container min-h-0 flex-1 overflow-y-auto overscroll-contain px-5 py-5 outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
        >
          <div className="grid gap-x-6 gap-y-6 @xl:grid-cols-2">
            {groups.map(({ group, detail, entries }) => (
              <section
                key={group}
                aria-labelledby={`shortcuts-${group}`}
                className="min-w-0"
              >
                <div className="mb-2 flex items-baseline justify-between gap-3 px-0.5">
                  <h3
                    id={`shortcuts-${group}`}
                    className="text-[13px] font-medium text-foreground"
                  >
                    {group}
                  </h3>
                  {detail ? (
                    <span className="truncate text-xs text-muted-foreground">
                      {detail}
                    </span>
                  ) : null}
                </div>
                <ul className="divide-y divide-border overflow-hidden rounded-lg border border-border">
                  {entries.map((entry) => (
                    <li
                      key={entry.id}
                      className="flex min-h-9 items-center justify-between gap-4 px-3 py-1 text-[13px]"
                    >
                      <span className="min-w-0 truncate text-foreground">
                        {entry.title}
                      </span>
                      {/* Alternatives, not one chord: without a separator `tab`
                          and `shift tab` read as a single four-key press. */}
                      <span className="flex shrink-0 items-center gap-1.5">
                        {resolveBindings(entry.bindings, platform).map(
                          (binding, index) => (
                            <Fragment key={binding}>
                              {index > 0 ? (
                                <span className="text-[11px] text-muted-foreground">
                                  or
                                </span>
                              ) : null}
                              <KeyCombo
                                binding={binding}
                                platform={platform}
                                digitRange={entry.digitIndex}
                                capClassName={KEYCAP}
                              />
                            </Fragment>
                          ),
                        )}
                      </span>
                    </li>
                  ))}
                </ul>
              </section>
            ))}
          </div>
        </div>

        <DialogFooter className="mx-0 mb-0 items-center justify-between gap-3 border-border px-5 py-3">
          <p className="text-xs text-muted-foreground">
            Rows in the ⌘K palette show their keys too.
          </p>
          <DialogClose render={<Button variant="outline" size="sm" />}>
            Done
          </DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
