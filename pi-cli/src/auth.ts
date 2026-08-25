/**
 * Who pays for this terminal.
 *
 * pi keeps credentials per provider, so the question is "does the provider this
 * terminal will use have credentials, and of which kind": an OAuth
 * subscription login, or an API key. `pi auth check --json` answers for one
 * provider, and `--credentials` is deliberately NOT passed — the page needs the
 * KIND of credential, never the credential.
 */

import type { IIIClient } from 'iii-sdk';
import { exec } from './host.js';

export type Billing = 'subscription' | 'api-key' | 'none' | 'unknown';

export type AuthStatus = {
  billing: Billing;
  label: string;
  detail: string;
  provider: string;
  ready: boolean;
  reason: string;
};

type PiAuthCheck = {
  status?: string;
  provider?: string;
  reason?: string;
  credential?: { type?: string; source?: string };
  type?: string;
  source?: string;
};

function unknown(provider: string): AuthStatus {
  return {
    billing: 'unknown',
    label: 'billing unknown',
    detail: `pi auth check did not answer for ${provider} on the terminal host`,
    provider,
    ready: false,
    reason: '',
  };
}

export function readStatus(raw: string, provider: string): AuthStatus {
  let parsed: PiAuthCheck;
  try {
    parsed = JSON.parse(raw) as PiAuthCheck;
  } catch {
    return unknown(provider);
  }

  const name = parsed.provider ?? provider;
  const ready = parsed.status === 'ready';
  const reason = parsed.reason ?? '';
  const kind = parsed.credential?.type ?? parsed.type ?? '';

  if (!ready) {
    return {
      billing: 'none',
      label: `${name}: not signed in`,
      detail:
        reason === 'credentials_not_configured'
          ? `No ${name} credentials on the terminal host: run /login in this terminal, or set the provider's API key.`
          : `pi cannot use ${name} yet${reason ? `: ${reason}` : ''}.`,
      provider: name,
      ready: false,
      reason,
    };
  }

  // pi reports the credential kind when it has one; an OAuth credential is a
  // subscription login, an api key is metered.
  const billing: Billing = kind.includes('oauth')
    ? 'subscription'
    : kind.includes('api')
      ? 'api-key'
      : 'unknown';
  const label =
    billing === 'subscription'
      ? `${name} subscription`
      : billing === 'api-key'
        ? `${name} API key billing`
        : `${name} ready`;
  return {
    billing,
    label,
    detail:
      billing === 'unknown'
        ? `pi has ${name} credentials on the terminal host; their kind was not reported.`
        : `Billing to the ${name} ${billing === 'subscription' ? 'subscription login' : 'API key'} on the terminal host.`,
    provider: name,
    ready: true,
    reason: '',
  };
}

export function registerAuth(
  iii: IIIClient,
  current: () => { executable: string; provider: string; env: Record<string, string> },
): void {
  iii.registerFunction(
    'pi-cli::auth::status',
    async () => {
      const { executable, provider, env } = current();
      if (!executable) return unknown(provider);
      // Same host as the session, so the answer is the session's answer.
      // `pi auth check` exits 1 when a provider is not ready and still prints
      // the JSON that says why, so the exit code is not the answer here.
      try {
        const result = await exec(iii, `${executable} auth check --provider ${provider} --json`, {
          env,
          timeoutMs: 20_000,
        });
        const raw = result.stdout.trim() || result.stderr.trim();
        return raw ? readStatus(raw, provider) : unknown(provider);
      } catch (err) {
        console.warn(`pi auth check failed: ${String(err)}`);
        return unknown(provider);
      }
    },
    {
      description:
        "Who pays for a pi terminal session on this host: the provider's subscription login, its API key, or nothing yet. Read from `pi auth check` on the terminal host; never returns the credential itself.",
      request_format: { type: 'object', properties: {} },
      response_format: {
        type: 'object',
        required: ['billing', 'label', 'detail'],
        properties: {
          billing: { enum: ['subscription', 'api-key', 'none', 'unknown'] },
          label: { type: 'string' },
          detail: { type: 'string' },
          provider: { type: 'string' },
          ready: { type: 'boolean' },
          reason: { type: 'string' },
        },
      },
      metadata: { trace_hidden: true },
    },
  );
}
