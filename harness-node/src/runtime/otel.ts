/**
 * Harness OTel wiring.
 *
 * Ports `workers/harness/src/otel.rs` to Node:
 *   - `initHarnessOtel` boots the iii-sdk OTel SDK once per process.
 *   - `instrumentHandler` wraps a function handler so every invocation
 *     opens a span tagged with `iii.session.id` / `iii.message.id` /
 *     `iii.function.id` and propagates the same IDs via baggage so
 *     downstream `iii.trigger` calls inherit them.
 *
 * Without this, the engine's `engine::traces::group_by` returns empty
 * groups for Session / Message / Function in the traces UI — see
 * `engine/src/workers/observability/mod.rs:917-920`.
 */

import { SpanKind, context as otelContext, propagation } from '@opentelemetry/api';
import {
  type OtelConfig,
  SeverityNumber,
  currentSpanIsRecording,
  getLogger as getOtelLogger,
  initOtel,
  recordSpanEvent,
  redactAndTruncate,
  resolveMaxBytesFromEnv,
  setCurrentSpanAttribute,
  setCurrentSpanError,
  withSpan,
} from 'iii-sdk/telemetry';
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

// =============================================================================
// OTel log bridge
// =============================================================================
//
// The Rust harness gets log -> OTel bridging for free via
// `tracing-opentelemetry::OtelLogsLayer` (every `tracing::info!` /
// `warn!` / `error!` becomes both a span event and an OTel log,
// correlated to the active trace via context propagation). The Node
// port uses `pino`, which writes to stderr only. Without this bridge,
// the `LOGS` tab in the traces UI is silent on every harness-node
// span — even though the same workers in Rust populate it.
//
// We keep `pino` for stderr (devs still see logs in their terminal)
// and ALSO emit each log to OTel via `iii-sdk/telemetry`'s
// `getLogger()`. The OTel logger auto-correlates the log to the
// currently active span (trace_id + span_id), so logs land in the
// engine's `engine::logs::list` storage AND under the SpanOtelLogsTab
// for the right span. When `initOtel` hasn't run, `getLogger()`
// returns null and the OTel side is a quiet no-op.

type Bindings = Record<string, unknown>;
type AttrValue = string | number | boolean;

/**
 * OTel attribute values must be primitives (or arrays of primitives).
 * Errors become their message; objects/arrays become JSON. `null` and
 * `undefined` are dropped so they don't pollute the LOGS tab UI.
 */
function toAttrValue(v: unknown): AttrValue | undefined {
  if (v === null || v === undefined) return undefined;
  if (typeof v === 'string' || typeof v === 'number' || typeof v === 'boolean') return v;
  if (v instanceof Error) return v.message;
  try {
    return JSON.stringify(v);
  } catch {
    return String(v);
  }
}

/**
 * Flatten the `(bindings, data)` pair into a single OTel-safe attribute
 * map. Bindings (from `child(...)`) come first; `data` keys override
 * binding keys with the same name. Non-object `data` is stashed under
 * an `iii.log.data` key so it isn't lost.
 */
function flattenAttrs(bindings: Bindings, data: unknown): Record<string, AttrValue> {
  const out: Record<string, AttrValue> = {};
  for (const [k, v] of Object.entries(bindings)) {
    const av = toAttrValue(v);
    if (av !== undefined) out[k] = av;
  }
  if (data && typeof data === 'object' && !(data instanceof Error)) {
    for (const [k, v] of Object.entries(data as Record<string, unknown>)) {
      const av = toAttrValue(v);
      if (av !== undefined) out[k] = av;
    }
  } else if (data !== undefined) {
    const av = toAttrValue(data);
    if (av !== undefined) out['iii.log.data'] = av;
  }
  return out;
}

function emitOtelLog(
  severityNumber: SeverityNumber,
  severityText: string,
  bindings: Bindings,
  msg: string,
  data?: unknown,
): void {
  const otelLogger = getOtelLogger();
  if (!otelLogger) return;
  otelLogger.emit({
    severityNumber,
    severityText,
    body: msg,
    attributes: flattenAttrs(bindings, data),
  });
}

function wrap(p: pino.Logger, bindings: Bindings = {}): Logger {
  return {
    info: (m, d) => {
      p.info(d ?? {}, m);
      emitOtelLog(SeverityNumber.INFO, 'INFO', bindings, m, d);
    },
    warn: (m, d) => {
      p.warn(d ?? {}, m);
      emitOtelLog(SeverityNumber.WARN, 'WARN', bindings, m, d);
    },
    error: (m, d) => {
      p.error(d ?? {}, m);
      emitOtelLog(SeverityNumber.ERROR, 'ERROR', bindings, m, d);
    },
    debug: (m, d) => {
      p.debug(d ?? {}, m);
      emitOtelLog(SeverityNumber.DEBUG, 'DEBUG', bindings, m, d);
    },
    child: (b) => wrap(p.child(b), { ...bindings, ...b }),
  };
}

export const logger: Logger = wrap(baseLogger);

// =============================================================================
// ID extraction (port of harness/src/otel.rs:53-284)
// =============================================================================

/** 128 chars caps response headers and span attrs; mirrors `MAX_ID_LEN`. */
const MAX_ID_LEN = 128;

export interface HarnessIds {
  sessionId?: string;
  messageId?: string;
}

/** Printable ASCII only. Rejects CR/LF/control bytes so IDs can't poison logs. */
function isSafeIdChar(code: number): boolean {
  return code >= 0x20 && code <= 0x7e;
}

function validId(value: unknown): string | undefined {
  if (typeof value !== 'string') return undefined;
  if (value.length === 0 || value.length > MAX_ID_LEN) return undefined;
  for (let i = 0; i < value.length; i++) {
    if (!isSafeIdChar(value.charCodeAt(i))) return undefined;
  }
  return value;
}

/**
 * Pull `session_id` / `message_id` out of a function payload.
 *
 * Resolution order mirrors `extract_body_or_top_level_ids` with a
 * bridge-trigger fallback into `body.payload.*` and a final baggage
 * fallback so inner wrappers see IDs set by outer wrappers.
 *
 * Supported wrap depths (in order):
 *   1. `root.session_id` / `root.message_id`              (direct call)
 *   2. `root.body.session_id` / `root.body.message_id`    (HTTP envelope)
 *   3. `root.body.payload.session_id` / `.message_id`     (`harness::call` bridge)
 *
 * Deeper wraps are intentionally NOT followed — if a future bridge
 * adds another layer, lift IDs to the outer envelope or extend this
 * function. Falling through to the baggage layer is the safety net
 * for nested calls but breaks when the outermost handler is the one
 * missing the IDs.
 */
export function extractHarnessIds(input: unknown): HarnessIds {
  if (typeof input !== 'object' || input === null) {
    return baggageFallback();
  }
  const root = input as Record<string, unknown>;
  const body =
    root.body && typeof root.body === 'object' ? (root.body as Record<string, unknown>) : root;

  let sessionId = validId(body.session_id);
  let messageId = validId(body.message_id);

  // bridge::trigger shape — outer absent, look in nested payload.
  if (sessionId === undefined && messageId === undefined) {
    const payload = body.payload;
    if (payload && typeof payload === 'object') {
      const inner = payload as Record<string, unknown>;
      sessionId = validId(inner.session_id);
      messageId = validId(inner.message_id);
    }
  }

  // Baggage fallback so nested handlers inherit IDs set by outer ones.
  if (sessionId === undefined) sessionId = getBaggage('iii.session.id');
  if (messageId === undefined) messageId = getBaggage('iii.message.id');

  return { sessionId, messageId };
}

function baggageFallback(): HarnessIds {
  return {
    sessionId: getBaggage('iii.session.id'),
    messageId: getBaggage('iii.message.id'),
  };
}

function getBaggage(key: string): string | undefined {
  const bag = propagation.getBaggage(otelContext.active());
  return bag?.getEntry(key)?.value;
}

// =============================================================================
// Span wrap (port of harness/src/otel.rs:328-403 `run_in_span`)
// =============================================================================

export type Handler<TIn = unknown, TOut = unknown> = (input: TIn) => Promise<TOut>;

/**
 * Wrap a function handler so every invocation runs inside an OTel span
 * tagged with `iii.session.id` / `iii.message.id` / `iii.function.id`,
 * and the same IDs are pushed as baggage so downstream `iii.trigger`
 * calls inherit them.
 *
 * Also mirrors `iii-sdk-rust`'s auto-instrumentation (see
 * `motia/sdk/packages/rust/iii/src/iii.rs:1689-1773`) by recording
 * three lifecycle span events on the wrap span:
 *
 *   - `iii.invocation.input`  — redacted+truncated input JSON
 *   - `iii.invocation.output` — redacted+truncated result JSON (or
 *                                {error: ...} on failure)
 *   - `exception`             — only on throw, with type/message/stack
 *
 * This is what populates the "EVENTS" tab in the traces UI for
 * harness-node-emitted spans. Without it, harness-node spans look
 * "empty" compared to Rust harness spans, because the Rust harness
 * gets the same events for free via `iii-sdk-rust` (and via the
 * `tracing-opentelemetry::OpenTelemetryLayer` bridge, which doesn't
 * exist on the Node side because `pino` doesn't bridge to OTel).
 *
 * Gated on `III_DISABLE_TRACE_PAYLOADS` to match the Rust
 * `iii-sdk` semantics (set to `1` to suppress payload capture).
 *
 * The wrap is a no-op (just forwards to the handler) when OTel has not
 * been initialized — `currentSpanIsRecording()` returns false and the
 * `withSpan` itself is cheap.
 */
export function instrumentHandler<TIn = unknown, TOut = unknown>(
  functionId: string,
  handler: Handler<TIn, TOut>,
): Handler<TIn, TOut> {
  const spanName = `harness.${functionId}`;
  return async (input: TIn): Promise<TOut> => {
    return withSpan(spanName, { kind: SpanKind.SERVER }, async () => {
      const ids = extractHarnessIds(input);

      if (currentSpanIsRecording()) {
        if (ids.sessionId !== undefined) {
          setCurrentSpanAttribute('iii.session.id', ids.sessionId);
        }
        if (ids.messageId !== undefined) {
          setCurrentSpanAttribute('iii.message.id', ids.messageId);
        }
        setCurrentSpanAttribute('iii.function.id', functionId);
      }

      // Match the iii-sdk-rust pattern: emit input/output events with
      // the (redacted, truncated) payload JSON. The env-var override
      // matches the Rust SDK's `III_DISABLE_TRACE_PAYLOADS` knob.
      const tracePayloads = !isTracePayloadsDisabled();
      const payloadMaxBytes = resolveMaxBytesFromEnv();

      if (tracePayloads) {
        const { json, truncated } = redactAndTruncate(input, payloadMaxBytes);
        recordSpanEvent('iii.invocation.input', {
          'iii.payload.json': json,
          'iii.payload.truncated': truncated,
        });
      }

      // Propagate the IDs as baggage so child spans created by downstream
      // `iii.trigger` calls inherit them via BaggageSpanProcessor.
      const entries: Record<string, { value: string }> = {
        'iii.function.id': { value: functionId },
      };
      if (ids.sessionId !== undefined) entries['iii.session.id'] = { value: ids.sessionId };
      if (ids.messageId !== undefined) entries['iii.message.id'] = { value: ids.messageId };

      const baggage = propagation.createBaggage(entries);
      const ctxWithBaggage = propagation.setBaggage(otelContext.active(), baggage);

      try {
        const result = await otelContext.with(ctxWithBaggage, () => handler(input));
        if (tracePayloads) {
          const { json, truncated } = redactAndTruncate(result, payloadMaxBytes);
          recordSpanEvent('iii.invocation.output', {
            'iii.payload.json': json,
            'iii.payload.truncated': truncated,
            'iii.payload.ok': true,
          });
        }
        return result;
      } catch (err) {
        // Record both the iii-sdk-style output event (so the EVENTS
        // tab shows the failure inline next to the success path) AND
        // the standard OTel `exception` event (so the ERRORS tab can
        // pick it up). Mirrors `iii.rs:1750-1773`.
        const message = err instanceof Error ? err.message : String(err);
        const stack = err instanceof Error ? (err.stack ?? '') : '';
        const exceptionType =
          err instanceof Error && err.constructor?.name ? err.constructor.name : 'Error';
        if (tracePayloads) {
          const { json, truncated } = redactAndTruncate({ error: message }, payloadMaxBytes);
          recordSpanEvent('iii.invocation.output', {
            'iii.payload.json': json,
            'iii.payload.truncated': truncated,
            'iii.payload.ok': false,
          });
        }
        recordSpanEvent('exception', {
          'exception.type': exceptionType,
          'exception.message': message,
          'exception.stacktrace': stack,
        });
        setCurrentSpanError(message);
        throw err;
      }
    });
  };
}

function isTracePayloadsDisabled(): boolean {
  const raw = process.env.III_DISABLE_TRACE_PAYLOADS;
  if (!raw) return false;
  const v = raw.toLowerCase();
  return v === '1' || v === 'true';
}

// =============================================================================
// OTel boot
// =============================================================================

let otelInitialized = false;

/**
 * Initialize the iii-sdk OTel SDK once per process and register a
 * `BaggageSpanProcessor` so allowlisted baggage entries (`iii.session.id`
 * etc.) are stamped onto every span as attributes.
 *
 * Subsequent calls are no-ops, which is important when several workers
 * are loaded into a single process via `src/index.ts`.
 */
export function initHarnessOtel(serviceName: string, engineWsUrl?: string): void {
  if (otelInitialized) return;
  otelInitialized = true;

  const config: OtelConfig = {
    serviceName,
  };
  if (engineWsUrl) config.engineWsUrl = engineWsUrl;

  try {
    initOtel(config);
  } catch (err) {
    // The iii-sdk's initOtel may throw if `OTEL_ENABLED=false` or the
    // env is unsupported. We don't want a failed OTel init to take down
    // a worker — fall back to no-op spans (handler still runs, just no
    // group-by traces).
    otelInitialized = false;
    logger.warn('initHarnessOtel failed; spans will be no-ops', { err: String(err) });
    return;
  }

  // iii-sdk's `initOtel` auto-registers `BaggageSpanProcessor` (see
  // iii-sdk@0.12.0 dist/telemetry-system-CFshsuNv.mjs:1171), so allowlisted
  // baggage entries (`iii.session.id` / `iii.message.id` /
  // `iii.function.id`) are stamped onto every span as attributes
  // automatically. No further wiring needed here.
}

/** Reset for tests. Do not call in production. */
export function _resetOtelForTests(): void {
  otelInitialized = false;
}
