import type { SecretKeyFieldProps } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'
import { credentialsFromProviderList } from '../../page'
import { LlmRouterConfigForm } from './index'

// The real components come from the running Console's import map; these
// stand-ins render just enough structure to read what the form passes down.
vi.mock('@iii-dev/console-ui', () => {
  const Pass = ({ children }: { children?: ReactNode }) => <div>{children}</div>
  return {
    Button: ({ children }: { children?: ReactNode }) => <button type="button">{children}</button>,
    Input: (props: { value?: string; type?: string; 'aria-label'?: string }) => (
      <input aria-label={props['aria-label']} type={props.type} value={props.value} readOnly />
    ),
    Select: () => <select />,
    SettingsList: Pass,
    SettingsSection: ({ children, title }: { children?: ReactNode; title?: string }) => (
      <section aria-label={title}>{children}</section>
    ),
    SettingsRow: ({
      label,
      description,
      control,
    }: {
      label?: ReactNode
      description?: ReactNode
      control?: ReactNode
    }) => (
      <div>
        {label}
        {description}
        {control}
      </div>
    ),
    Switch: () => <input type="checkbox" readOnly />,
  }
})

const schema = {
  type: 'object',
  properties: {
    providers: {
      type: 'object',
      properties: {
        kimi: {
          type: 'object',
          properties: { api_key: { type: 'string', writeOnly: true, format: 'password' } },
        },
      },
    },
  },
}

function StubSecretField(props: SecretKeyFieldProps) {
  return (
    <output
      data-name={props.name}
      data-value={props.value ?? ''}
      data-consumers={props.consumers.join(',')}
      data-environment={String(props.environment ?? false)}
      data-error={props.status?.error ?? ''}
    />
  )
}

describe('provider keys in the router form', () => {
  it('hands the key to the Console secret field under the variable the provider declares', () => {
    const html = renderToStaticMarkup(
      <LlmRouterConfigForm
        id="llm-router"
        schema={schema}
        value={{ providers: { kimi: { api_key: 'secret://MOONSHOT_API_KEY' } } }}
        onChange={() => undefined}
        secretField={StubSecretField}
        credentials={
          new Map([
            ['kimi', { envVar: 'MOONSHOT_API_KEY', connected: false, error: 'secret MOONSHOT_API_KEY not found' }],
          ])
        }
      />,
    )
    expect(html).toContain('data-name="MOONSHOT_API_KEY"')
    expect(html).toContain('data-value="secret://MOONSHOT_API_KEY"')
    expect(html).toContain('data-consumers="llm-router"')
    // The router resolves env://NAME too, so the field may offer .env.
    expect(html).toContain('data-environment="true"')
    expect(html).toContain('data-error="secret MOONSHOT_API_KEY not found"')
    expect(html).not.toContain('type="password"')
  })

  it('hands an env:// reference to the field as it is', () => {
    const html = renderToStaticMarkup(
      <LlmRouterConfigForm
        id="llm-router"
        schema={schema}
        value={{ providers: { kimi: { api_key: 'env://MOONSHOT_API_KEY' } } }}
        onChange={() => undefined}
        secretField={StubSecretField}
        credentials={new Map([['kimi', { envVar: 'MOONSHOT_API_KEY', connected: true, source: 'secret' }]])}
      />,
    )
    expect(html).toContain('data-value="env://MOONSHOT_API_KEY"')
  })

  it('keeps the plain key input on a Console without the secret field', () => {
    const html = renderToStaticMarkup(
      <LlmRouterConfigForm
        id="llm-router"
        schema={schema}
        value={{ providers: { kimi: { api_key: 'sk-literal' } } }}
        onChange={() => undefined}
      />,
    )
    expect(html).toContain('type="password"')
    expect(html).toContain('plain-text secret will be stored')
  })

  it('shows no key field for a provider that signs in on its own', () => {
    const html = renderToStaticMarkup(
      <LlmRouterConfigForm
        id="llm-router"
        schema={{
          type: 'object',
          properties: {
            providers: {
              type: 'object',
              properties: {
                'claude-code': {
                  type: 'object',
                  properties: { api_key: { type: 'string', writeOnly: true } },
                },
              },
            },
          },
        }}
        value={{}}
        onChange={() => undefined}
        secretField={StubSecretField}
        credentials={new Map([['claude-code', { connected: false }]])}
      />,
    )
    expect(html).not.toContain('data-name=')
    expect(html).toContain('signs in on its own')
  })

  it('reads each provider key variable and status from router::provider::list', () => {
    const credentials = credentialsFromProviderList({
      providers: [
        {
          id: 'anthropic',
          credential_env_var: 'ANTHROPIC_API_KEY',
          configured: true,
          credential_source: 'secret',
        },
        { id: 'claude-code', configured: false },
        { nope: true },
      ],
    })
    expect(credentials.get('anthropic')).toEqual({
      envVar: 'ANTHROPIC_API_KEY',
      connected: true,
      source: 'secret',
      error: undefined,
    })
    expect(credentials.get('claude-code')?.envVar).toBeUndefined()
    expect(credentials.size).toBe(2)
  })
})
