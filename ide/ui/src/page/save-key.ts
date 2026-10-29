/* Save (⌘S, Ctrl+S off a Mac): the pure half. Monaco binds nothing to
   the chord, so the editor pane is its only owner: one handler on the
   pane, which the editor's keys bubble into, and the header's save icon
   calls the same `save`. React-free so the tests import it. */

import type { KeyLike, QuickOpenPlatform } from './quick-open'

/** Whether a keystroke is the save chord. Either modifier counts on any
    platform, as it always has: the browser's own Save Page sits on the
    same chord, and the pane cancels it while focus is inside. */
export function isSaveKey(event: KeyLike): boolean {
  return (event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's'
}

/** How the chord prints in the save icon's tooltip. */
export function saveKeyLabel(platform: QuickOpenPlatform): string {
  return platform === 'mac' ? '⌘S' : 'Ctrl+S'
}

/** The chord for `aria-keyshortcuts`, in the reader's platform. */
export function saveKeyAria(platform: QuickOpenPlatform): string {
  return platform === 'mac' ? 'Meta+S' : 'Control+S'
}
