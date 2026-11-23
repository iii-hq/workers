import { afterEach, describe, expect, it } from 'vitest'
import {
  CAPABILITY_FUNCTIONS,
  capabilitiesFromCatalog,
  getChatCapabilities,
  setChatCapabilitiesForTests,
} from './chat-capabilities'

describe('chat capabilities (spec §0.2)', () => {
  afterEach(() => setChatCapabilitiesForTests(null))

  it('turns each feature on only when its function is in the catalogue', () => {
    const caps = capabilitiesFromCatalog({
      functions: [
        { function_id: 'session::patch-meta' },
        { function_id: 'harness::projects::create' },
        { function_id: 'harness::projects::list' },
        { function_id: 'harness::context-items' },
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
