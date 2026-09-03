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
import { exec, quote } from './host.js';

export type Billing = 'subscription' | 'api-key' | 'none' | 'unknown';

export type ProviderStatus = {
  provider: string;
  ready: boolean;
  /** 'subscription' for an OAuth login, 'api-key' for a key, '' when unsaid. */
  kind: '' | 'subscription' | 'api-key';
  reason: string;
};

export type AuthStatus = {
  billing: Billing;
  label: string;
  detail: string;
  /** The first ready provider, kept so an older page still reads one name. */
  provider: string;
  ready: boolean;
  reason: string;
  /** Every provider pi holds credentials for, ready or not. */
  providers: ProviderStatus[];
};

type PiAuthCheck = {
  status?: string;
  provider?: string;
  reason?: string;
  authType?: string;
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
    providers: [],
  };
}

/** One provider's answer from `pi auth check --provider X --json`. */
export function readProvider(raw: string, provider: string): ProviderStatus {
  let parsed: PiAuthCheck;
  try {
    parsed = JSON.parse(raw) as PiAuthCheck;
  } catch {
    return { provider, ready: false, kind: '', reason: 'unreadable answer' };
  }
  const name = parsed.provider ?? provider;
  const raw_kind = parsed.credential?.type ?? parsed.type ?? parsed.authType ?? '';
  // pi says `oauth` for a subscription login and `api_key` for a metered key.
  const kind = raw_kind.includes('oauth')
    ? 'subscription'
    : raw_kind.includes('api')
      ? 'api-key'
      : '';
  return {
    provider: name,
    ready: parsed.status === 'ready',
    kind,
    reason: parsed.reason ?? '',
  };
}

/**
 * What the badge says when several providers are configured.
 *
 * pi is not one account: a terminal can hold an OpenAI key, an Anthropic
 * subscription, and three providers that were half set up and left. Reporting
 * only the configured default called a working terminal "not signed in", which
 * is worse than saying nothing. So the badge names every provider that is
 * ready, and says plainly when none are.
 */
export function summarize(providers: ProviderStatus[]): AuthStatus {
  const ready = providers.filter((entry) => entry.ready);
  if (ready.length === 0) {
    const tried = providers.map((entry) => entry.provider).join(', ');
    return {
      billing: 'none',
      label: 'no provider signed in',
      detail: tried
        ? `pi has no usable credentials on the terminal host (checked ${tried}): run /login in this terminal, or set a provider's API key.`
        : "pi has no credentials on the terminal host: run /login in this terminal, or set a provider's API key.",
      provider: '',
      ready: false,
      reason: providers[0]?.reason ?? 'credentials_not_configured',
      providers,
    };
  }

  const named = (entry: ProviderStatus) =>
    entry.kind === 'subscription'
      ? `${entry.provider} (subscription)`
      : entry.kind === 'api-key'
        ? `${entry.provider} (API key)`
        : entry.provider;
  // A subscription is the answer worth surfacing when both kinds are present:
  // it is the one with a plan behind it rather than a per-token bill.
  const billing: Billing = ready.some((entry) => entry.kind === 'subscription')
    ? 'subscription'
    : ready.some((entry) => entry.kind === 'api-key')
      ? 'api-key'
      : 'unknown';
  const label =
    ready.length === 1
      ? named(ready[0])
      : `${ready.length} providers · ${ready.map((e) => e.provider).join(', ')}`;
  const idle = providers.filter((entry) => !entry.ready);
  return {
    billing,
    label,
    detail: `pi can use ${ready.map(named).join(', ')} on the terminal host.${
      idle.length ? ` Not ready: ${idle.map((entry) => entry.provider).join(', ')}.` : ''
    } Which one a session spends depends on the model it runs.`,
    provider: ready[0].provider,
    ready: true,
    reason: '',
    providers,
  };
}

/**
 * Which providers to ask about. pi keeps one entry per configured provider in
 * its auth store, and that file is the only list of what a person actually set
 * up — `pi auth check` answers for one provider at a time and has no "list"
 * form. The store is read for NAMES only; whether each one works is still pi's
 * answer, never a credential read from disk.
 *
 * An unreadable store falls back to the configured provider, which is what
 * this worker asked about before.
 */
export async function listProviders(
  iii: IIIClient,
  fallback: string,
  env: Record<string, string>,
): Promise<string[]> {
  try {
    const result = await exec(iii, `sh -c ${JSON.stringify('cat "$HOME/.pi/agent/auth.json"')}`, {
      env,
      timeoutMs: 15_000,
    });
    const parsed = JSON.parse(result.stdout || '{}') as Record<string, unknown>;
    const names = Object.keys(parsed).filter((name) => name.length > 0);
    return names.length > 0 ? names : [fallback];
  } catch {
    return [fallback];
  }
}

export function registerAuth(
  iii: IIIClient,
  current: () => { executable: string; provider: string; env: Record<string, string> },
): void {
  iii.registerFunction(
    'pi::auth::status',
    async () => {
      const { executable, provider, env } = current();
      if (!executable) return unknown(provider);
      // Same host as the session, so the answer is the session's answer.
      // `pi auth check` exits 1 when a provider is not ready and still prints
      // the JSON that says why, so the exit code is not the answer here.
      try {
        const names = await listProviders(iii, provider, env);
        const answers = await Promise.all(
          names.map(async (name): Promise<ProviderStatus> => {
            try {
              const result = await exec(
                iii,
                `${quote(executable)} auth check --provider ${quote(name)} --json`,
                { env, timeoutMs: 20_000 },
              );
              const raw = result.stdout.trim() || result.stderr.trim();
              return raw
                ? readProvider(raw, name)
                : { provider: name, ready: false, kind: '', reason: 'no answer' };
            } catch (err) {
              return { provider: name, ready: false, kind: '', reason: String(err) };
            }
          }),
        );
        return summarize(answers);
      } catch (err) {
        console.warn(`pi auth check failed: ${String(err)}`);
        return unknown(provider);
      }
    },
    {
      description:
        'Check which providers a pi terminal on this host can use. Reports the credential kind, a subscription login or an API key, read from `pi auth check`; never returns the credential itself.',
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
          providers: {
            type: 'array',
            items: {
              type: 'object',
              properties: {
                provider: { type: 'string' },
                ready: { type: 'boolean' },
                kind: { enum: ['', 'subscription', 'api-key'] },
                reason: { type: 'string' },
              },
            },
          },
        },
      },
      metadata: { trace_hidden: true },
    },
  );
}
