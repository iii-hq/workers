/**
 * Commands a page contributes to the palette and, while its pane has the
 * focus, to the keyboard.
 *
 * Two registration points land here. A worker's setup script registers
 * commands for a page it owns (`host.commands.register`): those live exactly
 * as long as the script, torn down with its other registrations when the
 * worker's assets are removed on disconnect or hot reload. A mounted page
 * registers commands through `PageRenderProps.commands`: those live while the
 * page is mounted in a pane and may carry keys, which fire only while focus is
 * inside that pane. Either way a worker that is not connected has no rows and
 * no keys, without any presence plumbing of its own.
 */

import { useSyncExternalStore } from 'react'
import { shortcutPlatform } from '@/lib/keybindings/bindings'
import {
  resolveBindings,
  shortcutClaimReason,
} from '@/lib/keybindings/registry'
import type { PageCommand } from '@/types/injectable-ui'

export type PageCommandSource = 'worker' | 'page'

export interface RegisteredPageCommand {
  /** `${pageId}.${command.id}`: unique across pages, stable across renders. */
  key: string
  pageId: string
  /** The page's title when known at registration; the palette falls back to
   *  the registered page's title, then the id. */
  pageTitle?: string
  source: PageCommandSource
  /** The pane a page-level registration belongs to; keys fire only there. */
  paneId?: string
  /** Bindings for this platform, after the ones that collide with the
   *  console's own keys were refused. */
  bindings: readonly string[]
  command: PageCommand
}

let snapshot: readonly RegisteredPageCommand[] = []
const listeners = new Set<() => void>()

function emit(): void {
  for (const listener of [...listeners]) listener()
}

export interface PageCommandRegistration {
  pageId: string
  pageTitle?: string
  source: PageCommandSource
  paneId?: string
  commands: readonly PageCommand[]
}

/** Register a page's commands; the return value removes exactly those. */
export function registerPageCommands(
  registration: PageCommandRegistration,
  platform = shortcutPlatform(),
): () => void {
  const entries = registration.commands.map((command) =>
    toEntry(registration, command, platform),
  )
  const keys = new Set(entries.map((entry) => entry.key))
  const shadowed = snapshot.filter(
    (entry) =>
      keys.has(entry.key) &&
      entry.source === registration.source &&
      entry.paneId === registration.paneId,
  )
  if (shadowed.length > 0) {
    console.warn(
      `[iii-ui] page '${registration.pageId}' re-registered commands ${shadowed
        .map((entry) => `'${entry.command.id}'`)
        .join(', ')}; the newer registration wins`,
    )
  }
  snapshot = [
    ...snapshot.filter((entry) => !shadowed.includes(entry)),
    ...entries,
  ]
  emit()
  let removed = false
  return () => {
    if (removed) return
    removed = true
    snapshot = snapshot.filter((entry) => !entries.includes(entry))
    emit()
  }
}

function claimedBindings(
  registration: PageCommandRegistration,
  command: PageCommand,
  platform: ReturnType<typeof shortcutPlatform>,
): readonly string[] {
  // Only a mounted page may hold keys: they are scoped to its pane. A
  // worker-level command is a palette row, so the console's global keymap
  // stays the console's.
  if (registration.source !== 'page' || registration.paneId === undefined)
    return []
  if (command.shortcut === undefined) return []
  const wanted =
    typeof command.shortcut === 'string'
      ? [command.shortcut]
      : resolveBindings(command.shortcut, platform)
  return wanted.filter((binding) => {
    const reason = shortcutClaimReason(binding, platform)
    if (reason) {
      console.warn(
        `[iii-ui] page '${registration.pageId}' command '${command.id}' cannot bind '${binding}': ${reason}`,
      )
    }
    return reason === null
  })
}

function toEntry(
  registration: PageCommandRegistration,
  command: PageCommand,
  platform: ReturnType<typeof shortcutPlatform>,
): RegisteredPageCommand {
  return {
    key: `${registration.pageId}.${command.id}`,
    pageId: registration.pageId,
    pageTitle: registration.pageTitle,
    source: registration.source,
    paneId: registration.paneId,
    bindings: claimedBindings(registration, command, platform),
    command,
  }
}

export function getPageCommands(): readonly RegisteredPageCommand[] {
  return snapshot
}

/** The keyed commands of one pane, for the dispatcher. */
export function paneCommands(paneId: string): readonly RegisteredPageCommand[] {
  return snapshot.filter(
    (entry) => entry.paneId === paneId && entry.bindings.length > 0,
  )
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
  }
}

const EMPTY: readonly RegisteredPageCommand[] = []

export function usePageCommands(): readonly RegisteredPageCommand[] {
  return useSyncExternalStore(subscribe, getPageCommands, () => EMPTY)
}

/** Tests only. */
export function resetPageCommands(): void {
  snapshot = []
  emit()
}
