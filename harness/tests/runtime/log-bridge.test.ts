/**
 * pino -> OTel log bridge contract test.
 *
 * Mirrors the Rust harness behavior where every `tracing::info!` /
 * `warn!` / `error!` call lands BOTH on stderr (via the fmt layer)
 * AND in the engine's OTel log storage (via OtelLogsLayer). The Node
 * port keeps `pino` for stderr and routes the same call into the
 * iii-sdk's OTel logger.
 *
 * In tests `initOtel` isn't called, so `getLogger()` normally returns
 * null. We mock the iii-sdk telemetry module so we can observe the
 * `emit` calls without booting a live engine connection.
 */

import { beforeEach, describe, expect, it, vi } from 'vitest';

interface CapturedRecord {
  severityNumber: number;
  severityText: string;
  body: string;
  attributes: Record<string, unknown>;
}

const captured: CapturedRecord[] = [];

vi.mock('@iii-dev/observability', async () => {
  const actual =
    await vi.importActual<typeof import('@iii-dev/observability')>('@iii-dev/observability');
  return {
    ...actual,
    getLogger: () => ({
      emit: (record: CapturedRecord) => {
        captured.push(record);
      },
    }),
  };
});

// Imported AFTER the mock so the mock is in effect when the module
// resolves `getLogger`. `SeverityNumber` is re-exported from
// @iii-dev/observability so we don't need a direct dependency on
// @opentelemetry/api-logs in this package.
const { logger } = await import('../../src/runtime/otel.js');
const { SeverityNumber } = await import('@iii-dev/observability');

beforeEach(() => {
  captured.length = 0;
});

describe('logger -> OTel bridge', () => {
  it('emits an INFO log when logger.info is called', () => {
    logger.info('hello world');
    expect(captured).toHaveLength(1);
    expect(captured[0].severityNumber).toBe(SeverityNumber.INFO);
    expect(captured[0].severityText).toBe('INFO');
    expect(captured[0].body).toBe('hello world');
    expect(captured[0].attributes).toEqual({});
  });

  it('emits WARN / ERROR / DEBUG with the right severity', () => {
    logger.warn('careful');
    logger.error('boom');
    logger.debug('chatty');

    expect(captured.map((r) => [r.severityNumber, r.severityText])).toEqual([
      [SeverityNumber.WARN, 'WARN'],
      [SeverityNumber.ERROR, 'ERROR'],
      [SeverityNumber.DEBUG, 'DEBUG'],
    ]);
  });

  it('flattens a data object into log attributes', () => {
    logger.info('user signed in', {
      user_id: 'u-1',
      tier: 'pro',
      retries: 0,
      active: true,
    });
    expect(captured[0].attributes).toEqual({
      user_id: 'u-1',
      tier: 'pro',
      retries: 0,
      active: true,
    });
  });

  it('serializes nested objects via JSON.stringify so OTel-safe primitives are preserved', () => {
    logger.info('event', { payload: { nested: { value: 42 } } });
    expect(captured[0].attributes.payload).toBe('{"nested":{"value":42}}');
  });

  it('handles Error values by capturing only the message (not the full Error)', () => {
    logger.error('handler failed', { err: new Error('boom') });
    expect(captured[0].attributes.err).toBe('boom');
  });

  it('drops null / undefined attribute values rather than passing them to OTel', () => {
    logger.info('partial', { keep: 'yes', drop_null: null, drop_undef: undefined });
    expect(captured[0].attributes).toEqual({ keep: 'yes' });
  });

  it('stashes non-object data under `iii.log.data` so primitives are not lost', () => {
    logger.info('count', 42 as unknown as Record<string, unknown>);
    expect(captured[0].attributes['iii.log.data']).toBe(42);
  });

  it('child(bindings) merges bindings into every emitted log', () => {
    const child = logger.child({ worker: 'harness', request_id: 'r-1' });
    child.info('handled');
    expect(captured[0].attributes).toMatchObject({
      worker: 'harness',
      request_id: 'r-1',
    });
  });

  it('child bindings can be overridden by per-call data', () => {
    const child = logger.child({ stage: 'init' });
    child.warn('changed mid-flight', { stage: 'shutdown' });
    expect(captured[0].attributes.stage).toBe('shutdown');
  });

  it('nested child(...) stacks bindings', () => {
    const child = logger.child({ worker: 'turn-orchestrator' });
    const grandchild = child.child({ phase: 'assistant' });
    grandchild.info('streaming token');
    expect(captured[0].attributes).toMatchObject({
      worker: 'turn-orchestrator',
      phase: 'assistant',
    });
  });
});
