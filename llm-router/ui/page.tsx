/**
 * llm-router's injectable console UI: a custom configuration form for the
 * `llm-router` entry (provider credentials, routing heuristics, stream
 * budgets). Provider keys use the Console's shared `SecretKeyField` when it
 * has one — kept by the secrets worker, encrypted (`secret://NAME`) or in the
 * project's `.env` (`env://NAME`) — and the plain-text-secret detection
 * otherwise. Registered per the injectable-ui contract
 * (docs/sops/injectable-console-ui.md).
 */

import type { ConfigFormProps, Host, SecretKeyFieldProps } from '@iii-dev/console-ui'
import { type ComponentType, useEffect, useState } from 'react'
import { LlmRouterConfigForm, type ProviderCredential } from './src/configuration'

/** `router::provider::list` as the form needs it: each provider's key variable and status. */
export function credentialsFromProviderList(response: unknown): Map<string, ProviderCredential> {
  const rows = (response as { providers?: unknown } | null)?.providers
  const out = new Map<string, ProviderCredential>()
  for (const raw of Array.isArray(rows) ? rows : []) {
    const row = raw as Record<string, unknown> | null
    if (!row || typeof row.id !== 'string') continue
    out.set(row.id, {
      envVar: typeof row.credential_env_var === 'string' ? row.credential_env_var : undefined,
      connected: row.configured === true,
      source: typeof row.credential_source === 'string' ? row.credential_source : undefined,
      error: typeof row.credential_error === 'string' ? row.credential_error : undefined,
    })
  }
  return out
}

function RouterConfigForm({
  host,
  secretField,
  ...props
}: ConfigFormProps & { host: Host; secretField?: ComponentType<SecretKeyFieldProps> }) {
  const [credentials, setCredentials] = useState<ReadonlyMap<string, ProviderCredential>>(new Map())
  // Re-read after each save: a stored key changes what the router resolves.
  const saved = JSON.stringify(props.value ?? null)
  useEffect(() => {
    let cancelled = false
    void host.iii
      .trigger('router::provider::list', {})
      .then((response) => {
        if (!cancelled) setCredentials(credentialsFromProviderList(response))
      })
      .catch(() => undefined)
    return () => {
      cancelled = true
    }
  }, [host, saved])
  return <LlmRouterConfigForm {...props} secretField={secretField} credentials={credentials} />
}

export default function setup(host: Host) {
  const secretField = host.components?.SecretKeyField as ComponentType<SecretKeyFieldProps> | undefined
  host.configForms.register('llm-router', (props) => (
    <RouterConfigForm {...props} host={host} secretField={secretField} />
  ))
}
