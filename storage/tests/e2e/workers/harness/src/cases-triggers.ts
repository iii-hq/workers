// Trigger dispatch assertions. Each case drains buffered events first to
// avoid contamination from prior puts/deletes that arrived asynchronously.
//
// Rustfs delivers webhooks within ~100ms of the underlying mutation. The
// SQS path (S3 backend → MinIO webhook → bridge → ElasticMQ → worker
// SQS poller, 1-second long-poll) adds a few seconds of latency, so we
// pick a 10s window that's generous headroom for both. If the window
// expires without a match, the case fails with a useful message rather
// than hanging the whole harness.
import type { CaseContext, Provider, TestCase } from './cases.ts';

const EVENT_TIMEOUT_MS = 10_000;
const SILENCE_WINDOW_MS = 2_000;

async function flush(ctx: { resetEvents: () => void }): Promise<void> {
  // Brief grace period to absorb in-flight events from the previous case,
  // then drop the buffer so we measure only what THIS case produces.
  await new Promise((r) => setTimeout(r, 250));
  ctx.resetEvents();
}

interface TriggerScenario {
  name: string;
  run: (ctx: CaseContext) => Promise<void>;
}

// One scenario per behavioural assertion. The runner instantiates each
// scenario once per provider, so keys must be unique-per-case (provider
// is encoded in `ctx.bucket` but the key namespace is shared across runs
// against the same backend).
const SCENARIOS: readonly TriggerScenario[] = [
  {
    name: 'object-created fires on putObject',
    async run(ctx) {
      await flush(ctx);
      const key = 'harness/triggers/created-target';
      await ctx.call('storage::putObject', {
        bucket: ctx.bucket,
        key,
        body_base64: ctx.b64('created-event-body'),
        content_type: 'text/plain',
      });
      const ev = await ctx.waitForEvent('created', key, EVENT_TIMEOUT_MS);
      if (ev.bucket !== ctx.bucket) {
        throw new Error(`event.bucket=${ev.bucket}, expected ${ctx.bucket}`);
      }
    },
  },
  {
    name: 'object-deleted fires on deleteObject',
    async run(ctx) {
      const key = 'harness/triggers/deleted-target';
      // Pre-create the object outside the assertion window so the
      // resulting object-created dispatch doesn't pollute the buffer
      // we're about to measure for the deleted event.
      await ctx.call('storage::putObject', {
        bucket: ctx.bucket,
        key,
        body_base64: ctx.b64('soon-to-be-deleted'),
        content_type: 'text/plain',
      });
      await flush(ctx);

      await ctx.call('storage::deleteObject', { bucket: ctx.bucket, key });
      await ctx.waitForEvent('deleted', key, EVENT_TIMEOUT_MS);
    },
  },
  {
    name: 'multiple puts produce at least one created event',
    async run(ctx) {
      // Backends may coalesce rapid same-key puts; we assert ">=1" rather
      // than "==2" so the test stays stable under future event-batching
      // changes.
      await flush(ctx);
      const key = 'harness/triggers/multi-put';
      for (const body of ['v1', 'v2-larger', 'v3']) {
        await ctx.call('storage::putObject', {
          bucket: ctx.bucket,
          key,
          body_base64: ctx.b64(body),
          content_type: 'text/plain',
        });
      }
      // First event is enough — coalescing is fine, dropping all events is not.
      await ctx.waitForEvent('created', key, EVENT_TIMEOUT_MS);
    },
  },
  {
    name: 'getObject does not produce trigger events',
    async run(ctx) {
      const key = 'harness/triggers/silence-probe';
      // Seed an object so the GETs succeed; do this before flush so its
      // own object-created event is drained out.
      await ctx.call('storage::putObject', {
        bucket: ctx.bucket,
        key,
        body_base64: ctx.b64('silence-probe'),
        content_type: 'text/plain',
      });
      await flush(ctx);

      // Multiple reads should not produce any events.
      for (let i = 0; i < 3; i++) {
        await ctx.call('storage::getObject', { bucket: ctx.bucket, key });
      }
      await ctx.expectSilence(SILENCE_WINDOW_MS);
    },
  },
  {
    name: 'rapid-fire 10 puts → 10 created events received',
    async run(ctx) {
      await flush(ctx);
      const baseKey = 'harness/triggers/rapid';
      const keys = Array.from({ length: 10 }, (_, i) => `${baseKey}-${i}`);
      // Sequential puts within a tight loop — distinct keys so coalescing
      // doesn't apply. Assert at-least-once delivery for every key.
      for (const key of keys) {
        await ctx.call('storage::putObject', {
          bucket: ctx.bucket, key, body_base64: ctx.b64('rapid'), content_type: 'text/plain',
        });
      }
      for (const key of keys) {
        await ctx.waitForEvent('created', key, EVENT_TIMEOUT_MS);
      }
    },
  },
];

/**
 * Build the full trigger-dispatch suite, fanned out across every provider
 * in `providers`. The runner is responsible for skipping providers whose
 * trigger registration failed (`triggerlessProviders`) — this builder
 * doesn't know which backends have working SQS / webhook plumbing.
 */
export function buildTriggerCases(providers: readonly Provider[]): TestCase[] {
  return providers.flatMap((provider) =>
    SCENARIOS.map((s) => ({
      name: `[${provider}] ${s.name}`,
      provider,
      run: s.run,
    })),
  );
}
