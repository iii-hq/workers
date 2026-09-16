import { readdirSync, readFileSync } from 'node:fs'
import { join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'
import { describe, expect, it } from 'vitest'

/**
 * Panel layouts respond to the pane, not the viewport (ade/skills/design-system.md,
 * "Numbers" and "Do / Don't"):
 * `@container` on the region and `@2xl:` / `@3xl:` / `@5xl:` inside. The
 * one legitimate viewport-wide switch is the console chrome's phone-vs-
 * desktop presentation at 640px: bottom sheet vs popover, the phone menu,
 * 16px inputs and 48px touch targets on phones, safe-area padding.
 *
 * This is a ratchet. A file may keep viewport utilities only while it is
 * listed here with a reason, and never more of them than it has today.
 */
const srcRoot = fileURLToPath(new URL('../', import.meta.url))

const TOUCH = 'phone chrome: 48px touch targets and 16px text below 640px'
const SHEET =
  'phone chrome: bottom sheet rows at 56px/16px vs desktop popover rows'

const viewportAllowlist: Record<string, { max: number; reason: string }> = {
  'App.tsx': {
    max: 16,
    reason:
      'phone chrome: the workspace strip snaps one panel at a time on phones, phone header vs desktop menu bar, dialog sized against the viewport',
  },
  'components/CommandPalette.tsx': {
    max: 31,
    reason:
      'phone chrome: full-screen on phones vs a centred dialog, keyboard hints only where a keyboard is',
  },
  'components/chat/ActiveSubagentChips.tsx': { max: 1, reason: TOUCH },
  'components/chat/AddProviderPanel.tsx': { max: 8, reason: SHEET },
  'components/chat/ChatPanel.tsx': { max: 3, reason: TOUCH },
  'components/chat/ChatView.tsx': {
    max: 6,
    reason: 'phone chrome: phone header hides the status dot; touch sizes',
  },
  'components/chat/Composer.tsx': {
    max: 4,
    reason: 'phone chrome: the toolbar folds into the one-line phone composer',
  },
  'components/chat/CopyMessageButton.tsx': { max: 1, reason: TOUCH },
  'components/chat/DirectoryPicker.tsx': {
    max: 94,
    reason: `${SHEET} (the sheet switches at 768px, see mobileSheet)`,
  },
  'components/chat/EmptyState.tsx': { max: 8, reason: TOUCH },
  'components/chat/Message.tsx': {
    max: 27,
    reason: 'phone chrome: hover-revealed actions need a pointer; 16px copy',
  },
  'components/chat/MessageList.tsx': { max: 4, reason: TOUCH },
  'components/chat/ModelPicker.tsx': { max: 22, reason: SHEET },
  'components/chat/ModelWaitingIndicator.tsx': { max: 2, reason: TOUCH },
  'components/chat/ProviderConfigurationPanel.tsx': { max: 2, reason: TOUCH },
  'components/chat/ProviderSettingsForm.tsx': { max: 15, reason: TOUCH },
  'components/chat/ReasoningEffortSlider.tsx': { max: 2, reason: TOUCH },
  'components/chat/SessionAddonsPicker.tsx': { max: 1, reason: TOUCH },
  'components/chat/SessionTriggers.tsx': { max: 3, reason: TOUCH },
  'components/chat/SystemNotice.tsx': { max: 29, reason: TOUCH },
  'components/chat/SystemPromptPicker.tsx': { max: 3, reason: TOUCH },
  'components/chat/engine/RegisterTriggerView.tsx': { max: 10, reason: TOUCH },
  'components/chat/harness/SpawnView.tsx': { max: 7, reason: TOUCH },
  'components/chat/sandbox/ErrorCard.tsx': { max: 18, reason: TOUCH },
  'components/function-trigger/FunctionTriggerCard.tsx': {
    max: 6,
    reason: TOUCH,
  },
  'components/permissions/FullModeConfirmDialog.tsx': {
    max: 8,
    reason: 'phone chrome: stacked 48px buttons in the phone dialog',
  },
  'components/sidebar/ConversationRow.tsx': { max: 1, reason: TOUCH },
  'components/sidebar/ConversationSidebar.tsx': { max: 4, reason: TOUCH },
  'components/trigger-activity/TriggerActivityCard.tsx': {
    max: 18,
    reason: TOUCH,
  },
  'components/trigger-activity/TriggerDetails.tsx': { max: 7, reason: TOUCH },
  'components/ui/ActivityMetadata.tsx': { max: 2, reason: TOUCH },
  'components/ui/ActivityStatus.tsx': { max: 2, reason: TOUCH },
  'components/ui/Badge.tsx': { max: 1, reason: TOUCH },
  'components/ui/BottomSheet.tsx': {
    max: 2,
    reason: 'phone chrome: the phone bottom sheet itself',
  },
  'components/ui/Dialog.tsx': { max: 3, reason: TOUCH },
  'components/ui/ImageViewer.tsx': { max: 5, reason: TOUCH },
  'components/ui/Input.tsx': { max: 2, reason: TOUCH },
  'components/ui/OpenDetailsAffordance.tsx': { max: 5, reason: TOUCH },
  'components/ui/PageChrome.tsx': { max: 3, reason: TOUCH },
  'components/ui/RawValueInput.tsx': {
    max: 2,
    reason: 'phone chrome: the input and its button stack as 44px rows',
  },
  'components/ui/Select.tsx': { max: 6, reason: TOUCH },
  'components/ui/Selector.tsx': { max: 2, reason: TOUCH },
  'components/ui/SettingsDeck.tsx': { max: 4, reason: TOUCH },
  'components/workspace/EmptyPane.tsx': { max: 2, reason: TOUCH },
  'components/workspace/TabStrip.tsx': { max: 7, reason: TOUCH },
  'components/workspace/pane-controls.tsx': {
    max: 5,
    reason: 'phone chrome: split handles and pane rails need a pointer',
  },
  'pages/Configuration/index.tsx': { max: 14, reason: TOUCH },
  'pages/Configuration/tabs/WorkersTab/WorkerEditor.tsx': {
    max: 1,
    reason: TOUCH,
  },
}

/** CSS files that may keep width media queries, with today's count. */
const cssMediaAllowlist: Record<string, { max: number; reason: string }> = {
  'index.css': {
    max: 3,
    reason:
      'phone chrome: the one-line phone composer, 16px editor text, side-by-side panel motion',
  },
  'styles/ui-recipes.css': {
    max: 4,
    reason:
      'phone chrome: touch heights, 48px switch hit area and 16px settings text',
  },
  'components/chat/ReasoningEffortSlider.css': {
    max: 1,
    reason: 'phone chrome: one row in the popover, stacked in the phone sheet',
  },
}

// A Tailwind variant prefix (`sm:px-3`, `max-sm:hidden`), not an object key
// (`sm: '...'`) and not a container variant (`@sm:`).
const VIEWPORT_UTILITY = /(?<![@\w])(?:max-)?(?:2xl|sm|md|lg|xl):(?=\S)/g
const WIDTH_MEDIA = /@media[^{]*\((?:min|max)-width/g

function walk(ext: string): string[] {
  return readdirSync(srcRoot, { recursive: true, encoding: 'utf8' })
    .filter((file) => file.endsWith(ext))
    .map((file) => relative(srcRoot, join(srcRoot, file)))
    .filter(
      (file) =>
        !file.startsWith('demo/') &&
        !file.endsWith('.stories.tsx') &&
        !file.endsWith('.test.tsx'),
    )
    .sort()
}

function count(file: string, pattern: RegExp): number {
  return readFileSync(join(srcRoot, file), 'utf8').match(pattern)?.length ?? 0
}

describe('Viewport breakpoint ratchet', () => {
  it('keeps viewport utilities to the allowlisted console chrome', () => {
    const offenders: string[] = []
    for (const file of walk('.tsx')) {
      const found = count(file, VIEWPORT_UTILITY)
      const allowed = viewportAllowlist[file]
      if (!allowed && found > 0) {
        offenders.push(`${file}: ${found} viewport utilities (use @container)`)
      } else if (allowed && found > allowed.max) {
        offenders.push(`${file}: ${found} > ${allowed.max} allowed`)
      } else if (allowed && found === 0) {
        offenders.push(`${file}: allowlist entry is stale, remove it`)
      }
    }
    expect(offenders).toEqual([])
  })

  it('keeps width media queries to the allowlisted chrome stylesheets', () => {
    const offenders: string[] = []
    for (const file of walk('.css')) {
      const found = count(file, WIDTH_MEDIA)
      const allowed = cssMediaAllowlist[file]
      if (!allowed && found > 0) {
        offenders.push(`${file}: ${found} width media queries (use @container)`)
      } else if (allowed && found > allowed.max) {
        offenders.push(`${file}: ${found} > ${allowed.max} allowed`)
      }
    }
    expect(offenders).toEqual([])
  })

  it('marks every kept file as phone chrome', () => {
    const unmarked = Object.keys(viewportAllowlist).filter(
      (file) =>
        !readFileSync(join(srcRoot, file), 'utf8').includes(
          'viewport: phone chrome',
        ),
    )
    expect(unmarked).toEqual([])
  })
})
