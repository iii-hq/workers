import { describe, expect, it, vi } from 'vitest'
import { defaultConfig } from '../src/config.js'
import { CONTROLLER_PROFILE, ControllerSessions, controllerPolicy } from '../src/controller.js'
import { controlPlaneFixture } from '../src/fixtures.js'
import type { ControlPlaneConfig, ControlPlaneData } from '../src/types.js'

function fake(config: ControlPlaneConfig) {
  const state = new Map<string, unknown>()
  const sessions = new Set<string>()
  const call = vi.fn(async (id: string, input: Record<string, unknown> = {}) => {
    if (id === 'state::get') return state.get(String(input.key)) ?? null
    if (id === 'state::set') {
      state.set(String(input.key), input.value)
      return input.value
    }
    if (id === 'session::get') return sessions.has(String(input.session_id)) ? { meta: {} } : null
    if (id === 'session::ensure') {
      sessions.add(String(input.session_id))
      return { meta: {} }
    }
    if (id === 'harness::send') return { turn_id: 't1' }
    return {}
  })
  const data = {
    ...controlPlaneFixture,
    projects: [{ id: 'assistente', name: 'Assistente' }],
  } as unknown as ControlPlaneData
  const controllers = new ControllerSessions({
    iii: { call: call as never },
    config: () => config,
    read: async () => data,
  })
  return {
    call,
    controllers,
    sessions,
    sent: () => call.mock.calls.filter(([id]) => id === 'harness::send'),
  }
}

const withModel = (patch: Partial<ControlPlaneConfig> = {}): ControlPlaneConfig => {
  const config = defaultConfig()
  return {
    ...config,
    summary: { ...config.summary, model: 'claude-code::claude-code/sonnet' },
    ...patch,
  }
}

describe('controller chat sessions', () => {
  it('creates one project controller with the profile, its model and an unrestricted policy, then reuses it', async () => {
    const f = fake(withModel())
    const first = await f.controllers.open('assistente')
    expect(first.created).toBe(true)
    expect(first.session_id).toMatch(/^lead-assistente-[0-9a-f-]{36}$/)
    const ensure = f.call.mock.calls.find(([id]) => id === 'session::ensure')?.[1]
    expect(ensure).toMatchObject({
      title: 'Controller · Assistente',
      kind: 'user',
      metadata: {
        origin: 'control-plane-controller',
        project_id: 'assistente',
        model: 'claude-code::claude-code/sonnet',
      },
    })
    const [, send] = f.sent()[0]
    expect(send).toMatchObject({
      session_id: first.session_id,
      provider: 'claude-code',
      model: 'claude-code/sonnet',
      options: { agent: CONTROLLER_PROFILE, functions: { allow: controllerPolicy('assistente') } },
    })
    expect(controllerPolicy('assistente')).toEqual(['*'])

    expect(await f.controllers.open('assistente')).toEqual({
      session_id: first.session_id,
      created: false,
    })
    expect(f.sent()).toHaveLength(1)
  })

  it('has every function in All projects too, and uses its own model over the summary one', async () => {
    const f = fake(withModel({ controller: { model: 'openai::gpt-6', reasoning: 'high' } }))
    await f.controllers.open('all')
    const [, send] = f.sent()[0]
    expect(send).toMatchObject({
      provider: 'openai',
      model: 'gpt-6',
      options: { thinking_level: 'high' },
    })
    expect(controllerPolicy('all')).toEqual(['*'])
  })

  it('replaces a deleted session, shares a concurrent open, and asks for a model when none is set', async () => {
    const f = fake(withModel())
    const [a, b] = await Promise.all([
      f.controllers.open('assistente'),
      f.controllers.open('assistente'),
    ])
    expect(a).toEqual(b)
    f.sessions.clear()
    const next = await f.controllers.open('assistente')
    expect(next.created).toBe(true)
    expect(next.session_id).not.toBe(a.session_id)

    await expect(fake(defaultConfig()).controllers.open('assistente')).rejects.toThrow(
      'Pick a controller model',
    )
  })
})
