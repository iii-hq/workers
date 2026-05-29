/**
 * Config for the `web::fetch` worker. Pulled from the `web:` section
 * of the harness config.yaml; missing keys fall back to safe defaults.
 *
 * The numeric caps here are the HARD ceilings — a caller can ask for
 * less (smaller timeout, smaller max_bytes) per-call, but never more.
 * That keeps a runaway agent from spending the whole budget on one
 * fetch.
 */

import { getNumber, getSection, getString } from '../runtime/config.js';

export type WebConfig = {
  /** Hard ceiling on per-request timeout. */
  max_timeout_ms: number;
  /** Hard ceiling on response body bytes accepted before truncation. */
  max_response_bytes: number;
  /** Max redirect hops before giving up. */
  max_redirects: number;
  /** UA we identify ourselves as. */
  user_agent: string;
  /**
   * Allow requests to loopback addresses (`127.0.0.0/8`, `::1`,
   * `localhost`). Defaults to `true` because the iii harness itself
   * binds workers like `iii-http` to `localhost:3111` and the agent's
   * own smoke-test workflow targets them. Flip to `false` for
   * non-dev deployments where loopback hosts sensitive services
   * (admin UIs, debug ports, cloud agents). Other private ranges
   * (RFC1918, link-local incl. AWS metadata, ULA, multicast) stay
   * blocked unconditionally.
   */
  allow_loopback: boolean;
};

const DEFAULTS: WebConfig = {
  max_timeout_ms: 30_000,
  max_response_bytes: 5 * 1024 * 1024,
  max_redirects: 5,
  user_agent: 'iii-harness/0.1 (+web::fetch)',
  allow_loopback: true,
};

function getBoolean(cfg: Record<string, unknown>, key: string, fallback: boolean): boolean {
  const v = cfg[key];
  return typeof v === 'boolean' ? v : fallback;
}

export function loadWebConfig(cfg: Record<string, unknown>): WebConfig {
  const section = getSection(cfg, 'web');
  return {
    max_timeout_ms: getNumber(section, 'max_timeout_ms', DEFAULTS.max_timeout_ms),
    max_response_bytes: getNumber(section, 'max_response_bytes', DEFAULTS.max_response_bytes),
    max_redirects: getNumber(section, 'max_redirects', DEFAULTS.max_redirects),
    user_agent: getString(section, 'user_agent', DEFAULTS.user_agent),
    allow_loopback: getBoolean(section, 'allow_loopback', DEFAULTS.allow_loopback),
  };
}
