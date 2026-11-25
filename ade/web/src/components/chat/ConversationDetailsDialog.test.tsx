// @vitest-environment jsdom

import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/Tooltip'
import type { ChatProject } from '@/lib/backend/projects'
import type { ConversationDetailsDraft } from '@/lib/conversation-details'
import { ConversationDetailsDialog } from './ConversationDetailsDialog'
;(
  globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
).IS_REACT_ACT_ENVIRONMENT = true

const NOW = 1_000_000

const conversation = {
  title: 'Webhook retries',
  description: 'Backoff with jitter for failed deliveries, then a PR',
  descriptionBy: 'agent' as const,
  descriptionAtMs: NOW - 120_000,
  projectId: 'prj_web',
  sessionMetadata: {
    description_history: [
      {
        text: 'Mapping every webhook sender',
        by: 'agent',
        at_ms: NOW - 540_000,
      },
      { text: 'My own words', by: 'user', at_ms: NOW - 660_000 },
    ],
  },
}

const projects: ChatProject[] = [
  {
    id: 'prj_web',
    name: 'acme-web',
    color: 'blue',
    folders: ['/w'],
    created_at_ms: 0,
    updated_at_ms: 0,
  },
]

let root: Root | null = null
let host: HTMLElement | null = null

function render(
  props: Partial<Parameters<typeof ConversationDetailsDialog>[0]> = {},
) {
  const onSave = vi.fn<(draft: ConversationDetailsDraft) => Promise<void>>(() =>
    Promise.resolve(),
  )
  const onOpenChange = vi.fn()
  host = document.createElement('div')
  document.body.appendChild(host)
  root = createRoot(host)
  act(() => {
    root?.render(
      <TooltipProvider delayDuration={0}>
        <ConversationDetailsDialog
          open
          conversation={conversation}
          onOpenChange={onOpenChange}
          onSave={onSave}
          projects={projects}
          agentSwitch
          now={NOW}
          {...props}
        />
      </TooltipProvider>,
    )
  })
  return { onSave, onOpenChange }
}

const dialog = () =>
  document.body.querySelector('[role="dialog"]') as HTMLElement
const field = (name: string) =>
  dialog().querySelector(`[name="${name}"]`) as
    | HTMLInputElement
    | HTMLTextAreaElement
const button = (text: string) =>
  [...dialog().querySelectorAll('button')].find(
    (b) => b.textContent?.trim() === text,
  ) as HTMLButtonElement | undefined

function type(el: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const proto =
    el instanceof HTMLTextAreaElement
      ? HTMLTextAreaElement.prototype
      : HTMLInputElement.prototype
  const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set
  act(() => {
    setter?.call(el, value)
    el.dispatchEvent(new Event('input', { bubbles: true }))
  })
}

afterEach(() => {
  act(() => root?.unmount())
  host?.remove()
  root = null
})

describe('Conversation details (§1.3.5)', () => {
  it('shows every element of the spec, with the exact copy', () => {
    render()
    const text = dialog().textContent ?? ''
    expect(text).toContain('Conversation details')
    expect(text).toContain(
      'Yours. The agent names a new conversation once, from its first message; it never renames one you named.',
    )
    expect(text).toContain('by agent · 2m')
    expect(text).toContain('Let the agent keep it up to date')
    expect(text).toContain(
      'It updates the description inside the turn it is already running, never with a turn of its own. Editing it yourself turns this off.',
    )
    expect(text).toContain('Recent descriptions')
    expect(text).toContain('Mapping every webhook sender')
    expect(text).toContain('you')
    expect(text).toContain('Project')
    expect(field('conversation-title').value).toBe('Webhook retries')
  })

  it('hides the switch and the project without them', () => {
    render({ agentSwitch: false, projects: undefined })
    const text = dialog().textContent ?? ''
    expect(text).not.toContain('Let the agent keep it up to date')
    expect(dialog().querySelector('[aria-label="Project"]')).toBeNull()
  })

  it('typing a description turns the agent off and saves it all at once', async () => {
    const { onSave, onOpenChange } = render()
    type(field('conversation-title'), 'Retries')
    type(field('conversation-description'), 'Written by me')
    // The pill goes: the text is no longer the agent's.
    expect(dialog().textContent).not.toContain('by agent')
    const toggle = dialog().querySelector('[role="switch"]')
    expect(toggle?.getAttribute('aria-checked')).toBe('false')
    await act(async () => {
      button('Save')?.click()
    })
    expect(onSave).toHaveBeenCalledWith({
      title: 'Retries',
      description: 'Written by me',
      agentKeepsIt: false,
      projectId: 'prj_web',
    })
    expect(onOpenChange).toHaveBeenCalledWith(false)
  })

  it("a recent description goes into the field as the user's edit", () => {
    render()
    act(() => {
      ;[...dialog().querySelectorAll('button')]
        .find((b) => b.textContent?.includes('My own words'))
        ?.click()
    })
    expect(field('conversation-description').value).toBe('My own words')
  })

  it('counts from 120 characters and keeps the dialog open on an error', async () => {
    const failing = vi.fn(() => Promise.reject(new Error('engine said no')))
    render({ onSave: failing })
    type(field('conversation-description'), 'x'.repeat(125))
    expect(dialog().textContent).toContain('125/140')
    await act(async () => {
      button('Save')?.click()
    })
    expect(dialog().querySelector('[role="alert"]')?.textContent).toContain(
      'engine said no',
    )
  })
})
