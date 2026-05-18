/**
 * Minimal logger + span shim. The Rust harness's `otel.rs` wraps every
 * registered function in an OTel span tagged with iii.session/message.id.
 * Phase 1 of the Node port ships a no-op `withSpan` and a pino-backed
 * logger; the OTel wiring is a deliberate follow-up (see plan §"Out of
 * scope").
 */

import pino from 'pino';

const baseLogger = pino({
  level: process.env.LOG_LEVEL ?? 'info',
  transport:
    process.env.NODE_ENV === 'production'
      ? undefined
      : { target: 'pino/file', options: { destination: 2 } },
});

export type Logger = {
  info(msg: string, data?: unknown): void;
  warn(msg: string, data?: unknown): void;
  error(msg: string, data?: unknown): void;
  debug(msg: string, data?: unknown): void;
  child(bindings: Record<string, unknown>): Logger;
};

function wrap(p: pino.Logger): Logger {
  return {
    info: (m, d) => p.info(d ?? {}, m),
    warn: (m, d) => p.warn(d ?? {}, m),
    error: (m, d) => p.error(d ?? {}, m),
    debug: (m, d) => p.debug(d ?? {}, m),
    child: (bindings) => wrap(p.child(bindings)),
  };
}

export const logger: Logger = wrap(baseLogger);

/** No-op span wrapper. Returns whatever the inner function returns. */
export async function withSpan<T>(
  _name: string,
  _attrs: Record<string, unknown> | undefined,
  fn: () => Promise<T>,
): Promise<T> {
  return fn();
}
