/**
 * Who pays for this terminal.
 *
 * `claude auth status` answers in JSON on the terminal host, and the answer has
 * a trap in it: when `ANTHROPIC_API_KEY` is set the CLI keeps reporting
 * `authMethod: "claude.ai"` (the account is still logged in) while BILLING
 * silently moves to the key — the CLI itself says "ANTHROPIC_API_KEY or another
 * auth source is set and takes precedence over your claude.ai login". So
 * `apiKeySource` is what decides the answer, not `authMethod`.
 *
 * The page shows the result, because "which plan is this spending" is not a
 * question anyone should have to answer by reading a config file.
 */

import type { IIIClient } from 'iii-sdk';
import { probe } from './host.js';

export type Billing = 'subscription' | 'api-key' | 'none' | 'unknown';

export type AuthStatus = {
  billing: Billing;
  /** One line for the status bar. */
  label: string;
  /** The longer form, for a tooltip or a log line. */
  detail: string;
  logged_in: boolean;
  method: string;
  email: string;
  organization: string;
  subscription_type: string;
  api_key_source: string;
};

type ClaudeAuthStatus = {
  loggedIn?: boolean;
  authMethod?: string;
  apiProvider?: string;
  apiKeySource?: string;
  email?: string | null;
  orgName?: string | null;
  subscriptionType?: string | null;
};

const UNKNOWN: AuthStatus = {
  billing: 'unknown',
  label: 'billing unknown',
  detail: 'claude auth status did not answer on the terminal host',
  logged_in: false,
  method: '',
  email: '',
  organization: '',
  subscription_type: '',
  api_key_source: '',
};

export function readStatus(raw: string): AuthStatus {
  let parsed: ClaudeAuthStatus;
  try {
    parsed = JSON.parse(raw) as ClaudeAuthStatus;
  } catch {
    return UNKNOWN;
  }

  const apiKeySource = parsed.apiKeySource ?? '';
  const subscription = parsed.subscriptionType ?? '';
  const email = parsed.email ?? '';
  const organization = parsed.orgName ?? '';
  const loggedIn = parsed.loggedIn === true;

  // An API key beats the login, so it is checked first.
  if (apiKeySource) {
    const via = apiKeySource === 'ANTHROPIC_API_KEY' ? apiKeySource : `${apiKeySource} (api key)`;
    return {
      billing: 'api-key',
      label: `API key billing · ${via}`,
      detail: loggedIn
        ? `Billing to the API key from ${via}. The claude.ai login${email ? ` (${email})` : ''} is signed in but NOT billed — unset the key to use the subscription.`
        : `Billing to the API key from ${via}.`,
      logged_in: loggedIn,
      method: parsed.authMethod ?? '',
      email,
      organization,
      subscription_type: subscription,
      api_key_source: apiKeySource,
    };
  }

  if (loggedIn) {
    const plan = subscription ? `${subscription} subscription` : 'Claude subscription';
    const who = [email, organization].filter(Boolean).join(' · ');
    return {
      billing: 'subscription',
      label: who ? `${plan} · ${who}` : plan,
      detail: `Billing to the ${plan}${who ? ` of ${who}` : ''}. No API key is set on the terminal host.`,
      logged_in: true,
      method: parsed.authMethod ?? '',
      email,
      organization,
      subscription_type: subscription,
      api_key_source: '',
    };
  }

  return {
    billing: 'none',
    label: 'not signed in',
    detail:
      'No credentials on the terminal host: run /login in this terminal, or set ANTHROPIC_API_KEY for API billing.',
    logged_in: false,
    method: parsed.authMethod ?? '',
    email,
    organization,
    subscription_type: subscription,
    api_key_source: '',
  };
}

export function registerAuth(iii: IIIClient, executable: () => string): void {
  iii.registerFunction(
    'claude::auth::status',
    async () => {
      const binary = executable();
      if (!binary) return UNKNOWN;
      // Same host as the session, so the answer is the session's answer.
      const raw = await probe(iii, `${binary} auth status`);
      return raw ? readStatus(raw) : UNKNOWN;
    },
    {
      description:
        'Who pays for a Claude terminal session on this host: a Claude subscription, an API key (which overrides the subscription login), or nothing yet. Read from `claude auth status` on the terminal host.',
      request_format: { type: 'object', properties: {} },
      response_format: {
        type: 'object',
        required: ['billing', 'label', 'detail'],
        properties: {
          billing: { enum: ['subscription', 'api-key', 'none', 'unknown'] },
          label: { type: 'string' },
          detail: { type: 'string' },
          logged_in: { type: 'boolean' },
          method: { type: 'string' },
          email: { type: 'string' },
          organization: { type: 'string' },
          subscription_type: { type: 'string' },
          api_key_source: { type: 'string' },
        },
      },
      metadata: { trace_hidden: true },
    },
  );
}
