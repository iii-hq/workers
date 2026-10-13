import type { FunctionTriggerMessage, Host } from '@iii-dev/console-ui'
import { isValidElement } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { createDirectoryTriggerRenderer } from './index'
import { KitApplyPreview, KitApplyView, KitPlanView, openKits } from './KitsViews'
import { isDirectoryFunction } from './parsers'

vi.mock('@iii-dev/console-ui', () => {
  const Null = () => null
  return {
    ActionLine: Null,
    Badge: Null,
    Button: Null,
    Card: Null,
    Chip: Null,
    Dialog: Null,
    DialogContent: Null,
    DialogDescription: Null,
    DialogTitle: Null,
    Eyebrow: Null,
    FileDiff: Null,
    MarkdownPreview: Null,
    MetaRow: Null,
    SegmentedControl: Null,
    Skeleton: Null,
    StatusPanel: Null,
    Tabs: Null,
    TabsContent: Null,
    TabsList: Null,
    TabsTrigger: Null,
  }
})

function message(functionId: string, over: Partial<FunctionTriggerMessage> = {}): FunctionTriggerMessage {
  return { id: 'm1', role: 'function-trigger', functionId, input: {}, createdAt: 0, ...over }
}

const planResponse = {
  status: 'planned',
  kit: 'acme/team',
  message: 'Install acme/team 1.0.0 is planned.',
  plan: {
    plan_id: 'kp_1',
    kind: 'install',
    kit: 'acme/team',
    from: null,
    to: '1.0.0',
    requested: 'latest',
    major: false,
    registry_url: '',
    workers: [],
    files: [],
    capabilities: {},
    functions: [],
    warnings: [],
    blocking: [],
    counts: { agents: 1, skills: 2, workers: 0, functions: 0 },
    created_at: '',
    expires_at: '',
    review: '',
  },
}

describe('kit chat cards', () => {
  it('claims the kit functions', () => {
    for (const id of [
      'directory::download-kit',
      'directory::kits::apply',
      'directory::kits::plan-update',
      'directory::kits::remove',
      'directory::kits::check-updates',
      'directory::kits::list',
    ]) {
      expect(isDirectoryFunction(id)).toBe(true)
    }
  })

  it('renders the planning card while download-kit runs and the plan card after', () => {
    const renderer = createDirectoryTriggerRenderer()
    const running = renderer.tryRenderRunning?.(message('directory::download-kit', { input: { kit: 'acme/team' } }))
    expect(isValidElement(running)).toBe(true)
    const done = renderer.tryRender(
      message('directory::download-kit', { input: { kit: 'acme/team' }, output: planResponse }),
    )
    expect(isValidElement(done) && done.type).toBe(KitPlanView)
  })

  it('previews the plan behind a pending apply approval, and nothing else', () => {
    const renderer = createDirectoryTriggerRenderer()
    const preview = renderer.tryRenderPreview?.(
      message('directory::kits::apply', { pendingApproval: true, input: { plan_id: 'kp_1' } }),
    )
    expect(isValidElement(preview) && preview.type).toBe(KitApplyPreview)
    expect(renderer.tryRenderPreview?.(message('directory::skills::update', { pendingApproval: true }))).toBeNull()
    // The pending state never reaches the regular render.
    expect(renderer.tryRender(message('directory::kits::apply', { pendingApproval: true }))).toBeNull()
  })

  it('renders apply results and leaves error outputs to the console', () => {
    const renderer = createDirectoryTriggerRenderer()
    const out = renderer.tryRender(
      message('directory::kits::apply', { input: { plan_id: 'kp_1' }, output: { error: 'D512 not_found' } }),
    )
    expect(out).toBeNull()
    const applied = renderer.tryRender(
      message('directory::kits::apply', {
        input: { plan_id: 'kp_1' },
        output: { status: 'applied', kit: 'acme/team', message: 'Installed', report: { agents: [] } },
      }),
    )
    expect(isValidElement(applied) && applied.type).toBe(KitApplyView)
  })

  it('opens the Kits segment on a plan when the console can place pages', () => {
    const open = vi.fn()
    expect(openKits({ panels: { open } } as unknown as Host, { plan_id: 'kp_1' })).toBe(true)
    expect(open).toHaveBeenCalledWith({ pageId: 'directory', context: { collection: 'kits', plan_id: 'kp_1' } })
    expect(openKits({} as Host, { plan_id: 'kp_1' })).toBe(false)
  })
})
