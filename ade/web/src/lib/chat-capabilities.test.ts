import { afterEach, describe, expect, it, vi } from 'vitest'
import {
  CAPABILITY_FUNCTIONS,
  capabilitiesFromCatalog,
  getChatCapabilities,
  refreshChatCapabilities,
  setChatCapabilitiesForTests,
} from './chat-capabilities'

const mocks = vi.hoisted(() => ({ call: vi.fn() }))
vi.mock('@/lib/iii-client', () => ({
  getIiiClient: () => Promise.resolve({ call: mocks.call }),
}))

/** The engine of chat-projects D37: the control plane is internal. */
const DEFAULT_LIST = [
  'harness::projects::list',
  'harness::set-description',
  'harness::projects::on-changed',
  'harness::chat-settings::on-changed',
]
const INTERNAL = [
  'harness::folders::list',
  'harness::folders::upsert',
  'harness::projects::create',
  'harness::projects::assign',
  'harness::context-items',
  'harness::chat-settings::get',
  'harness::chat-settings::set',
  'session::patch-meta',
  'harness::side-chat::open',
]
function engineList(payload: { include_internal?: boolean } = {}) {
  const ids = payload.include_internal
    ? [...DEFAULT_LIST, ...INTERNAL]
    : DEFAULT_LIST
  return { functions: ids.map((function_id) => ({ function_id })) }
}

describe('chat capabilities (spec §0.2)', () => {
  afterEach(() => setChatCapabilitiesForTests(null))

  it('reads the catalogue with internal functions, where the control plane is (D37)', async () => {
    mocks.call.mockReset()
    mocks.call.mockImplementation((_id: string, payload: object) =>
      Promise.resolve(engineList(payload)),
    )
    const caps = await refreshChatCapabilities()
    expect(mocks.call).toHaveBeenCalledWith('engine::functions::list', {
      include_internal: true,
    })
    expect(caps).toEqual({
      loaded: true,
      patchMeta: true,
      folders: true,
      projects: true,
      setDescription: true,
      contextItems: true,
      chatSettings: true,
      sideChats: true,
      harnessRetry: false,
      tmuxAccounts: false,
      tmuxCredential: false,
      claudeAccounts: false,
    })
  })

  it('turns each feature on only when its function is in the catalogue', () => {
    const caps = capabilitiesFromCatalog({
      functions: [
        { function_id: 'session::patch-meta' },
        { function_id: 'harness::projects::create' },
        { function_id: 'harness::projects::list' },
        { function_id: 'harness::context-items' },
        { function_id: 'harness::side-chat::open' },
      ],
    })
    expect(caps).toEqual({
      loaded: true,
      patchMeta: true,
      folders: false,
      projects: true,
      setDescription: false,
      contextItems: true,
      chatSettings: false,
      sideChats: true,
      harnessRetry: false,
      tmuxAccounts: false,
      tmuxCredential: false,
      claudeAccounts: false,
    })
  })

  it('reads harness::projects::list alone as the old folder catalogue, not projects', () => {
    const caps = capabilitiesFromCatalog({
      functions: [{ function_id: 'harness::projects::list' }],
    })
    expect(caps.projects).toBe(false)
  })

  it('names the function of every feature', () => {
    expect(CAPABILITY_FUNCTIONS).toMatchObject({
      folders: 'harness::folders::list',
      setDescription: 'harness::set-description',
      chatSettings: 'harness::chat-settings::get',
    })
  })

  it('turns Claude Code accounts on with retry, the credential and patch-meta together', () => {
    const ids = [
      'harness::retry',
      'tmux::accounts::list',
      'tmux::accounts::credential',
      'session::patch-meta',
    ]
    const with_ = (list: string[]) =>
      capabilitiesFromCatalog({
        functions: list.map((function_id) => ({ function_id })),
      }).claudeAccounts
    expect(with_(ids)).toBe(true)
    // Never written through set-meta: no patch-meta, no accounts.
    expect(with_(ids.filter((id) => id !== 'session::patch-meta'))).toBe(false)
    expect(with_(ids.filter((id) => id !== 'harness::retry'))).toBe(false)
    setChatCapabilitiesForTests({ harnessRetry: true, tmuxCredential: true })
    expect(getChatCapabilities().claudeAccounts).toBe(false)
    setChatCapabilitiesForTests({
      harnessRetry: true,
      tmuxCredential: true,
      patchMeta: true,
    })
    expect(getChatCapabilities().claudeAccounts).toBe(true)
  })

  it('is off until read, and a test can pin an answer', () => {
    expect(getChatCapabilities().loaded).toBe(false)
    setChatCapabilitiesForTests({ projects: true })
    expect(getChatCapabilities()).toMatchObject({
      loaded: true,
      projects: true,
      patchMeta: false,
    })
  })
})
