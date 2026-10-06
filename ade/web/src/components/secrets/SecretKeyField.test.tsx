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

const ENV_META = {
  ...META,
  ref: 'env://ANTHROPIC_API_KEY',
  store: 'env',
  fingerprint: undefined,
  location: '/p/.env',
}

/**
 * A secrets worker holding `stored` in its vault, sharing `shared` from the
 * env store, and seeing `found` on the machine.
 */
function secretsWorker({
  stored = null as typeof META | null,
  shared = null as typeof ENV_META | null,
  found = [] as { kind: string; location: string; hint: string }[],
  envFile = '/p/.env',
} = {}) {
  trigger.mockImplementation(
    async (fn: string, payload: { name?: string; store?: string }) => {
      if (fn === 'secrets::status') return { env_file: envFile }
      if (fn === 'secrets::get' && payload.store === 'env')
        return shared && payload.name === shared.name ? shared : null
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
      if (payload.store === 'env' && fn !== 'secrets::detect') return ENV_META
      if (fn === 'secrets::set' || fn === 'secrets::import') return META
      throw new Error(`unexpected ${fn}`)
    },
  )
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

/** Type into a controlled input the way a person does. */
async function type(input: HTMLInputElement, text: string) {
  const setter = Object.getOwnPropertyDescriptor(
    HTMLInputElement.prototype,
    'value',
  )?.set
  await act(async () => {
    setter?.call(input, text)
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
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

  it('offers to start the secrets worker again when it is not running', async () => {
    trigger.mockRejectedValue(new Error('function_not_found: secrets::get'))
    await render()
    expect(container.textContent).toContain('is not running')
    button('Start the secrets worker')
    expect(container.textContent).toContain('compose::add secrets')
  })

  it('keeps a key found in .env where it is, as an environment variable', async () => {
    secretsWorker({
      found: [{ kind: 'dotenv', location: '/p/.env', hint: 'sk-ant…9f2c' }],
    })
    const onChange = await render({ environment: true })
    // Encrypted stays the default.
    expect(container.textContent).toContain('secret://ANTHROPIC_API_KEY')
    await act(async () => button('Environment variable').click())
    expect(container.textContent).toContain(
      "Use ANTHROPIC_API_KEY from this project's .env",
    )
    expect(container.textContent).toContain('env://ANTHROPIC_API_KEY')
    await act(async () => button('Use variable').click())
    const access = trigger.mock.calls.find(([fn]) => fn === 'secrets::access')
    expect(access?.[1]).toEqual({
      name: 'ANTHROPIC_API_KEY',
      consumers: ['llm-router'],
      store: 'env',
    })
    expect(trigger.mock.calls.some(([fn]) => fn === 'secrets::set')).toBe(false)
    expect(onChange).toHaveBeenCalledWith('env://ANTHROPIC_API_KEY')
  })

  it('writes a pasted key to .env when the user prefers a variable', async () => {
    secretsWorker()
    const onChange = await render({ environment: true })
    await act(async () => button('Environment variable').click())
    const input = container.querySelector<HTMLInputElement>(
      'input[aria-label="ANTHROPIC_API_KEY"]',
    )
    if (!input) throw new Error('no key input')
    await type(input, 'sk-ant-pasted-into-dotenv-0000')
    await act(async () => button('Save to .env').click())
    const set = trigger.mock.calls.find(([fn]) => fn === 'secrets::set')
    expect(set?.[1]).toMatchObject({
      name: 'ANTHROPIC_API_KEY',
      value: 'sk-ant-pasted-into-dotenv-0000',
      consumers: ['llm-router'],
      store: 'env',
    })
    expect(onChange).toHaveBeenCalledWith('env://ANTHROPIC_API_KEY')
  })

  it('names the env file the secrets worker is configured with', async () => {
    secretsWorker({
      envFile: '/p/.env.staging',
      found: [
        { kind: 'dotenv', location: '/p/.env.staging', hint: 'sk-ant…9f2c' },
      ],
    })
    await render({ environment: true })
    await act(async () => button('Environment variable').click())
    expect(container.textContent).toContain(
      "Use ANTHROPIC_API_KEY from this project's .env.staging",
    )
    expect(container.textContent).toContain(
      'Paste a different key into .env.staging',
    )
    expect(container.textContent).toContain(
      "reads ANTHROPIC_API_KEY from this project's .env.staging",
    )
    expect(container.textContent).not.toMatch(/\.env(?!\.staging)/)
  })

  it('shows an env reference with where the variable is read from', async () => {
    secretsWorker({ shared: ENV_META })
    await render({ value: 'env://ANTHROPIC_API_KEY', environment: true })
    expect(container.textContent).toContain('env://ANTHROPIC_API_KEY')
    expect(container.textContent).toContain(
      "Environment variable · sk-ant…9f2c · from this project's .env",
    )
    expect(trigger).toHaveBeenCalledWith(
      'secrets::get',
      { name: 'ANTHROPIC_API_KEY', store: 'env' },
      expect.anything(),
    )
    button('Replace')
    button('Remove')
  })

  it('says when an env reference is not shared with the consumer yet', async () => {
    secretsWorker()
    await render({ value: 'env://ANTHROPIC_API_KEY', environment: true })
    expect(container.textContent).toContain('Not shared with llm-router yet')
  })

  it('offers no environment variable to a consumer that reads only secret://', async () => {
    secretsWorker({
      found: [{ kind: 'dotenv', location: '/p/.env', hint: 'sk-ant…9f2c' }],
    })
    await render()
    expect(container.textContent).not.toContain('Environment variable')
    expect(container.textContent).toContain(
      "Use the key from this project's .env",
    )
  })

  it('names the engine variable a dollar-brace value expands from', async () => {
    secretsWorker()
    // A configuration value the engine expands, not a template literal.
    const engineVar = ['$', '{ANTHROPIC_API_KEY}'].join('')
    await render({ value: engineVar, environment: true })
    expect(container.textContent).toContain(
      'Expanded by the engine from its ANTHROPIC_API_KEY variable',
    )
    button('Choose where to keep it')
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
