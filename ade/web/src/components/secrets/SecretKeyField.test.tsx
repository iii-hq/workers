// @vitest-environment jsdom
import { act } from 'react'
import { createRoot, type Root } from 'react-dom/client'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { SecretKeyField } from './SecretKeyField'

const trigger = vi.fn()

vi.mock('@/lib/iii-client', () => ({
  getIiiClient: async () => ({ trigger }),
}))

const META = {
  name: 'ANTHROPIC_API_KEY',
  ref: 'secret://ANTHROPIC_API_KEY',
  hint: 'sk-ant…9f2c',
  fingerprint: 'abc',
  consumers: ['llm-router'],
  created_at: new Date().toISOString(),
  updated_at: new Date().toISOString(),
}

let container: HTMLDivElement
let root: Root

beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true })
  container = document.createElement('div')
  document.body.append(container)
  root = createRoot(container)
  trigger.mockReset()
})

afterEach(async () => {
  await act(async () => root.unmount())
  container.remove()
})

/** A secrets worker holding `stored` and seeing `found` on the machine. */
function secretsWorker({
  stored = null as typeof META | null,
  found = [] as { kind: string; location: string; hint: string }[],
} = {}) {
  trigger.mockImplementation(async (fn: string, payload: { name?: string }) => {
    if (fn === 'secrets::get')
      return stored && payload.name === stored.name ? stored : null
    if (fn === 'secrets::detect') {
      return {
        results: [
          {
            name: 'ANTHROPIC_API_KEY',
            stored: Boolean(stored),
            stored_hint: stored?.hint,
            sources: found.map((source) => ({
              ...source,
              matches_stored: false,
            })),
          },
        ],
      }
    }
    if (fn === 'secrets::set' || fn === 'secrets::import') return META
    throw new Error(`unexpected ${fn}`)
  })
}

async function render(
  props: Partial<Parameters<typeof SecretKeyField>[0]> = {},
) {
  const onChange = props.onChange ?? vi.fn()
  await act(async () => {
    root.render(
      <SecretKeyField
        name="ANTHROPIC_API_KEY"
        value={undefined}
        consumers={['llm-router']}
        onChange={onChange}
        {...props}
      />,
    )
  })
  // Let the store read settle.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })
  return onChange
}

function button(name: string): HTMLButtonElement {
  const found = [...container.querySelectorAll('button')].find(
    (candidate) => candidate.textContent?.trim() === name,
  )
  if (!found) throw new Error(`no button ${name}: ${container.textContent}`)
  return found
}

describe('SecretKeyField', () => {
  it('shows a stored reference by name and masked hint, with replace and remove', async () => {
    secretsWorker({ stored: META })
    await render({
      value: 'secret://ANTHROPIC_API_KEY',
      status: { connected: true, source: 'secret' },
    })
    expect(container.textContent).toContain('secret://ANTHROPIC_API_KEY')
    expect(container.textContent).toContain('sk-ant…9f2c')
    expect(container.textContent).toContain('Connected')
    button('Replace')
    button('Remove')
  })

  it('imports a key found on the machine and writes only the reference', async () => {
    secretsWorker({
      found: [
        {
          kind: 'login_shell',
          location: 'login shell (zsh)',
          hint: 'sk-ant…0000',
        },
      ],
    })
    const onChange = await render()
    expect(container.textContent).toContain(
      'Use the key from your shell profile',
    )
    await act(async () => button('Save key').click())
    const importCall = trigger.mock.calls.find(
      ([fn]) => fn === 'secrets::import',
    )
    expect(importCall?.[1]).toMatchObject({
      name: 'ANTHROPIC_API_KEY',
      source: 'login_shell',
      consumers: ['llm-router'],
    })
    expect(onChange).toHaveBeenCalledWith('secret://ANTHROPIC_API_KEY')
  })

  it('moves a plain-text key into the store', async () => {
    secretsWorker()
    const onChange = await render({ value: 'sk-ant-plain-text-key' })
    expect(container.textContent).toContain(
      'Saved as plain text in configuration',
    )
    await act(async () => button('Move to secrets store').click())
    const setCall = trigger.mock.calls.find(([fn]) => fn === 'secrets::set')
    expect(setCall?.[1]).toMatchObject({ value: 'sk-ant-plain-text-key' })
    expect(onChange).toHaveBeenCalledWith('secret://ANTHROPIC_API_KEY')
  })

  it('says when the consumer reads the key from its own environment', async () => {
    secretsWorker()
    await render({ status: { connected: true, source: 'env' } })
    expect(container.textContent).toContain("from the worker's environment")
  })

  it('offers to add the secrets worker when it is not running', async () => {
    trigger.mockRejectedValue(new Error('function_not_found: secrets::get'))
    await render()
    expect(container.textContent).toContain('Add secrets worker')
    expect(container.textContent).toContain('compose::add secrets')
  })

  it('shows the error the consumer reports for a broken reference', async () => {
    secretsWorker()
    await render({
      value: 'secret://ANTHROPIC_API_KEY',
      status: {
        connected: false,
        error: 'secret ANTHROPIC_API_KEY not found in the secrets worker',
      },
    })
    expect(container.textContent).toContain('Not in the secrets store')
    expect(container.textContent).toContain('Needs attention')
    expect(container.textContent).toContain('not found in the secrets worker')
  })
})
