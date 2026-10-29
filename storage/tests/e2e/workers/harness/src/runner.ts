import { writeFileSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import type { IIIClient, ObserverHandle } from 'iii-sdk';
import {
  BUCKETS,
  LOCAL_BUCKET,
  buildSchemaReset,
  buildFunctionCases,
  type CaseContext,
  type ObjectEvent,
  type ObjectEventKind,
  type Provider,
  type TestCase,
} from './cases.ts';
import { buildEventCases } from './cases-events.ts';
import { ERROR_CASES } from './cases-errors.ts';
import {
  buildBodyShapeCases,
  buildKeyShapeCases,
  buildMetadataCases,
  buildErrorEnvelopeCases,
} from './cases-edge.ts';
import { buildProviderQuirkCases } from './cases-provider.ts';
import { buildConcurrencyCases } from './cases-concurrency.ts';
import { buildBucketCases, buildRangeCases } from './cases-buckets.ts';

interface CaseResult {
  case: string;
  provider?: Provider;
  status: 'PASS' | 'FAIL' | 'ERROR';
  error?: string;
  duration_ms: number;
}

export interface RunnerOptions {
  iii: IIIClient;
  reportPath: string;
  providers: Provider[];
  /**
   * Subset of `providers` whose object-event plumbing is wired. Observers,
   * probe, and case fan-out all use this list. Defaults to `providers`, so
   * callers that don't care opt into "fan out object events across every
   * selected provider" automatically. CI sets this to `local,s3` to keep
   * `r2` (no Cloudflare Queue plumbing here) out of object-event ERRORs that
   * would otherwise force a non-zero exit.
   */
  eventProviders?: Provider[];
  /** Optional case-name substring filter — when set, only matching cases run. */
  filterCase?: string;
}

const READY_TIMEOUT_MS = 30_000;
const READY_PROBE_INTERVAL_MS = 250;
/** How long an observer of an object function may take to become active. */
const OBSERVE_READY_TIMEOUT_MS = 10_000;

async function withTimeout<T>(promise: Promise<T>, ms: number, what: string): Promise<T> {
  let timer: NodeJS.Timeout | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${what} timed out after ${ms}ms`)), ms);
  });
  try {
    return await Promise.race([promise, timeout]);
  } finally {
    clearTimeout(timer);
  }
}

export class Runner {
  /**
   * Buffered events with their arrival timestamps. We don't drop events as
   * they're consumed — `waitForEvent` returns the first match by `(kind, key)`
   * but leaves later events alone — so a test that expects two events for
   * the same key still works. `resetEvents()` is the explicit clear.
   */
  private events: ObjectEvent[] = [];

  /** Resolvers waiting for an event to arrive that satisfies a predicate. */
  private waiters: Array<{
    match: (e: ObjectEvent) => boolean;
    resolve: (e: ObjectEvent) => void;
    reject: (err: Error) => void;
    timer: NodeJS.Timeout;
  }> = [];

  private observers: ObserverHandle[] = [];

  /**
   * Providers whose observers storage refused, or whose object-event
   * plumbing didn't deliver a probe event within the deadline. Their
   * object-event cases get ERROR-skipped instead of FAILing so a
   * half-broken cloud profile (e.g., MinIO up, bridge down) doesn't mask
   * unrelated regressions in the rest of the suite.
   */
  private eventlessProviders = new Set<Provider>();

  constructor(private opts: RunnerOptions) {}

  // ---------------- sink fns ----------------

  /**
   * The `storage::on-object-created` event shape (per
   * storage/src/events/object_created.rs::ObjectCreatedEvent):
   *   { bucket, key, size, etag, content_type?, created_at, raw_event_id }
   */
  onObjectCreated = async (payload: any): Promise<{ ack: boolean }> => {
    this.recordEvent({
      kind: 'created',
      bucket: String(payload?.bucket ?? ''),
      key: String(payload?.key ?? ''),
      size: typeof payload?.size === 'number' ? payload.size : undefined,
      etag: typeof payload?.etag === 'string' ? payload.etag : undefined,
      raw_event_id: String(payload?.raw_event_id ?? ''),
      received_at: Date.now(),
    });
    return { ack: true };
  };

  /**
   * The `storage::on-object-deleted` event shape (per
   * storage/src/events/object_deleted.rs::ObjectDeletedEvent):
   *   { bucket, key, deleted_at, raw_event_id }
   */
  onObjectDeleted = async (payload: any): Promise<{ ack: boolean }> => {
    this.recordEvent({
      kind: 'deleted',
      bucket: String(payload?.bucket ?? ''),
      key: String(payload?.key ?? ''),
      raw_event_id: String(payload?.raw_event_id ?? ''),
      received_at: Date.now(),
    });
    return { ack: true };
  };

  private recordEvent(ev: ObjectEvent): void {
    this.events.push(ev);
    // Resolve the OLDEST waiter that matches this event so FIFO ordering
    // is preserved when multiple waiters are queued for the same key.
    for (let i = 0; i < this.waiters.length; i++) {
      const w = this.waiters[i]!;
      if (w.match(ev)) {
        clearTimeout(w.timer);
        this.waiters.splice(i, 1);
        w.resolve(ev);
        return;
      }
    }
  }

  // ---------------- helpers exposed to cases ----------------

  private async callOnce(functionId: string, payload: unknown): Promise<any> {
    return await this.opts.iii.call<unknown, any>({ function_id: functionId, payload });
  }

  private async waitForEvent(
    kind: ObjectEventKind,
    key: string,
    timeoutMs: number,
  ): Promise<ObjectEvent> {
    // First, scan the existing buffer — events may have arrived before
    // the case got around to calling waitForEvent.
    for (const ev of this.events) {
      if (ev.kind === kind && ev.key === key) return ev;
    }
    return new Promise<ObjectEvent>((resolveP, rejectP) => {
      const match = (e: ObjectEvent) => e.kind === kind && e.key === key;
      const timer = setTimeout(() => {
        // Drop our entry from the waiters list so the timeout doesn't
        // accidentally resolve later if a stale event finally arrives.
        this.waiters = this.waiters.filter((w) => w.resolve !== resolveP);
        rejectP(
          new Error(
            `timed out after ${timeoutMs}ms waiting for ${kind} event on key="${key}"; ` +
              `buffer=${this.events.length} events: ${JSON.stringify(
                this.events.map((e) => ({ kind: e.kind, key: e.key })),
              )}`,
          ),
        );
      }, timeoutMs);
      this.waiters.push({ match, resolve: resolveP, reject: rejectP, timer });
    });
  }

  private async expectSilence(timeoutMs: number): Promise<void> {
    const startLen = this.events.length;
    await new Promise((r) => setTimeout(r, timeoutMs));
    const drift = this.events.length - startLen;
    if (drift > 0) {
      throw new Error(
        `expected silence for ${timeoutMs}ms, but received ${drift} new events: ` +
          JSON.stringify(this.events.slice(-Math.min(drift, 3)).map((e) => ({ kind: e.kind, key: e.key }))),
      );
    }
  }

  private resetEvents(): void {
    // Cancel any pending waiters: clearing the timer alone leaves the
    // promise hanging. Reject explicitly so the awaiting case fails fast
    // instead of stalling until the test runner times out the whole job.
    for (const w of this.waiters) {
      clearTimeout(w.timer);
      w.reject(new Error('waiter cancelled by resetEvents'));
    }
    this.waiters = [];
    this.events = [];
  }

  // ---------------- per-provider startup probe ----------------

  private async probeProvider(provider: Provider): Promise<{ ok: boolean; error?: string }> {
    const bucket = BUCKETS[provider];
    const key = `harness/.startup-probe-${provider}`;
    try {
      await this.callOnce('storage::putObject', {
        bucket, key, body_base64: Buffer.from('probe').toString('base64'), content_type: 'text/plain',
      });
      await this.callOnce('storage::deleteObject', { bucket, key });
      return { ok: true };
    } catch (e: any) {
      return { ok: false, error: e?.message ?? String(e) };
    }
  }

  // ---------------- case runner ----------------

  private async runCase(c: TestCase): Promise<CaseResult> {
    const provider: Provider = c.provider ?? 'local';
    const bucket = c.bucket ?? BUCKETS[provider];
    const ctx: CaseContext = {
      bucket,
      provider,
      iii: this.opts.iii,
      call: (id, payload) => this.callOnce(id, payload),
      b64: (s) => (typeof s === 'string' ? Buffer.from(s, 'utf8').toString('base64') : s.toString('base64')),
      fromB64: (s) => Buffer.from(s, 'base64').toString('utf8'),
      waitForEvent: (kind, key, timeoutMs) => this.waitForEvent(kind, key, timeoutMs),
      expectSilence: (timeoutMs) => this.expectSilence(timeoutMs),
      resetEvents: () => this.resetEvents(),
      expectError: async (fn, expectedCode) => {
        try {
          await fn();
        } catch (e: any) {
          const msg = e?.message ?? String(e);
          if (!msg.includes(expectedCode)) {
            throw new Error(`expected error code "${expectedCode}", got: ${msg}`);
          }
          return;
        }
        throw new Error(`expected throw with code "${expectedCode}", but call resolved`);
      },
    };
    const start = Date.now();
    try {
      await c.run(ctx);
      return { case: c.name, provider: c.provider, status: 'PASS', duration_ms: Date.now() - start };
    } catch (e: any) {
      return {
        case: c.name,
        provider: c.provider,
        status: 'FAIL',
        error: e?.message ?? String(e),
        duration_ms: Date.now() - start,
      };
    }
  }

  // ---------------- worker readiness ----------------

  /**
   * Probe storage with a getObject against a known-missing key. Once the
   * worker is connected and serving, the call returns OBJECT_NOT_FOUND
   * (an expected failure mode) — that proves the WS path is up. Connection
   * errors (engine still starting, worker not yet attached) bubble up as
   * different messages and we retry.
   */
  private async waitForStorageWorker(): Promise<void> {
    const deadline = Date.now() + READY_TIMEOUT_MS;
    let lastErr: unknown;
    while (Date.now() < deadline) {
      try {
        await this.callOnce('storage::getObject', {
          bucket: LOCAL_BUCKET,
          key: 'harness/.readiness-probe',
        });
        return; // unlikely (key shouldn't exist), but a 200 means worker is up
      } catch (e: any) {
        const msg = e?.message ?? String(e);
        if (msg.includes('OBJECT_NOT_FOUND') || msg.includes('UNKNOWN_BUCKET')) {
          return;
        }
        lastErr = e;
      }
      await new Promise((r) => setTimeout(r, READY_PROBE_INTERVAL_MS));
    }
    // Surface a useful worker-log hint instead of only the raw readiness error.
    const lastMsg = (lastErr as any)?.message ?? String(lastErr);
    const hint = lastMsg.includes('Function not found')
      ? '\n  hint: storage may have exited before registering RPCs. Run `iii worker logs storage` to inspect its startup error.'
      : '';
    throw new Error(
      `storage worker did not become ready within ${READY_TIMEOUT_MS}ms; last error: ${lastMsg}${hint}`,
    );
  }

  // ---------------- observer lifecycle ----------------

  private async observeObjectEvents(providers: readonly Provider[]): Promise<void> {
    // Observe both object functions with our two sink functions as targets,
    // once per provider. The 60s handler_timeout_ms is the storage default
    // (see storage/src/events/source.rs::DEFAULT_HANDLER_TIMEOUT_MS); we
    // leave it as-is since our handlers are near-instant.
    //
    // If a provider's bucket has no wired notifications source in the
    // worker config (e.g., scratch-s3 missing `notifications.sqs_queue_url`),
    // storage rejects the observer with CONFIG_ERROR
    // (storage/src/events/source.rs::admit), `ready()` rejects, and we tag
    // the provider as eventless so its cases ERROR-skip later.
    for (const provider of providers) {
      const bucket = BUCKETS[provider];
      const handles = [
        this.opts.iii.observe(
          { function_id: 'storage::on-object-created', args: { bucket } },
          'harness::on_object_created',
        ),
        this.opts.iii.observe(
          { function_id: 'storage::on-object-deleted', args: { bucket } },
          'harness::on_object_deleted',
        ),
      ];
      this.observers.push(...handles);
      try {
        await withTimeout(
          Promise.all(handles.map((h) => h.ready())),
          OBSERVE_READY_TIMEOUT_MS,
          `observing ${bucket}`,
        );
      } catch (e: unknown) {
        this.eventlessProviders.add(provider);
        const msg = e instanceof Error ? e.message : String(e);
        console.error(`[harness] observing object events failed for ${provider}: ${msg}`);
      }
    }
  }

  /**
   * Smoke-test the object-event path for each provider whose observers
   * storage accepted. Local skips the probe (in-process dispatch is
   * deterministic). Cloud providers do a put + waitForEvent and on timeout
   * are added to `eventlessProviders`. The probe key namespace is disjoint
   * from scenario keys so any leftover events drained at the start of the
   * first scenario via `flush()` won't confuse assertions.
   */
  private async probeEventPaths(providers: readonly Provider[]): Promise<void> {
    const PROBE_TIMEOUT_MS = 8_000;
    for (const provider of providers) {
      if (provider === 'local') continue;
      if (this.eventlessProviders.has(provider)) continue;
      const bucket = BUCKETS[provider];
      const key = `harness/.event-probe-${provider}-${Date.now()}`;
      try {
        await this.callOnce('storage::putObject', {
          bucket,
          key,
          body_base64: Buffer.from('probe').toString('base64'),
          content_type: 'text/plain',
        });
        await this.waitForEvent('created', key, PROBE_TIMEOUT_MS);
        // Best-effort cleanup; failures here aren't relevant to the probe.
        try {
          await this.callOnce('storage::deleteObject', { bucket, key });
        } catch {
          // ignored
        }
      } catch (e: unknown) {
        this.eventlessProviders.add(provider);
        const msg = e instanceof Error ? e.message : String(e);
        console.error(`[harness] object-event probe failed for ${provider}: ${msg}`);
      }
    }
  }

  /**
   * Dispose every observer and wait for the engine to confirm. Without
   * this, re-running the harness against the same long-running engine
   * leaves observers whose targets are about to disappear.
   */
  private async disposeObservers(): Promise<void> {
    const results = await Promise.allSettled(this.observers.map((h) => h.dispose()));
    for (const r of results) {
      if (r.status === 'rejected') {
        console.error(`[harness] dispose observer failed: ${r.reason}`);
      }
    }
    this.observers = [];
  }

  // ---------------- public entry ----------------

  async runAll(): Promise<{ pass: number; fail: number; errored: number; total: number; results: CaseResult[] }> {
    await this.waitForStorageWorker();
    // Object-event fan-out is constrained to providers that are both
    // selected (`opts.providers`) AND known to be wired
    // (`opts.eventProviders`). The intersection is computed once here and
    // reused below.
    const selected = new Set(this.opts.providers);
    const eventProviders = (this.opts.eventProviders ?? this.opts.providers).filter((p) =>
      selected.has(p),
    );
    await this.observeObjectEvents(eventProviders);

    const results: CaseResult[] = [];
    const useColor = process.stdout.isTTY === true;
    const GREEN = useColor ? '\x1b[32m' : '';
    const RED = useColor ? '\x1b[31m' : '';
    const YELLOW = useColor ? '\x1b[33m' : '';
    const RESET = useColor ? '\x1b[0m' : '';

    const record = (r: CaseResult): CaseResult => {
      const color = r.status === 'PASS' ? GREEN : r.status === 'FAIL' ? RED : YELLOW;
      const err = r.error ? ' — ' + r.error : '';
      console.log(`[harness] ${color}${r.status}${RESET} ${r.case} (${r.duration_ms}ms)${err}`);
      results.push(r);
      return r;
    };

    // Per-provider startup probe — ERROR-mark unreachable providers but continue.
    const providers = this.opts.providers;
    const downProviders = new Set<Provider>();
    for (const p of providers) {
      const probe = await this.probeProvider(p);
      if (!probe.ok) {
        downProviders.add(p);
        console.log(`[harness] ${YELLOW}ERROR${RESET} provider-probe[${p}]: ${probe.error}`);
      }
    }

    const accept = (c: TestCase): boolean =>
      !this.opts.filterCase || c.name.includes(this.opts.filterCase);

    const recordSkip = (c: TestCase): void => {
      record({
        case: c.name,
        provider: c.provider,
        status: 'ERROR',
        error: `provider ${c.provider} failed startup probe`,
        duration_ms: 0,
      });
    };

    // 1. Schema reset (uncounted; covers every provider).
    const reset = record(await this.runCase(buildSchemaReset(providers)));
    if (reset.status === 'FAIL') {
      await this.disposeObservers();
      return this.persist({ results, uncountedCases: ['schema-reset'] });
    }

    // 2. RPC suite.
    for (const c of buildFunctionCases(providers)) {
      if (!accept(c)) continue;
      if (c.provider && downProviders.has(c.provider)) { recordSkip(c); continue; }
      record(await this.runCase(c));
    }

    // 2b. Edge-case suite (a + b).
    const edgeBuilders = [
      buildBodyShapeCases,
      buildKeyShapeCases,
      buildMetadataCases,
      buildErrorEnvelopeCases,
    ];
    for (const build of edgeBuilders) {
      for (const c of build(providers)) {
        if (!accept(c)) continue;
        if (c.provider && downProviders.has(c.provider)) { recordSkip(c); continue; }
        record(await this.runCase(c));
      }
    }

    // 2c. Provider-quirk suite (f).
    for (const c of buildProviderQuirkCases(providers)) {
      if (!accept(c)) continue;
      if (c.provider && downProviders.has(c.provider)) { recordSkip(c); continue; }
      record(await this.runCase(c));
    }

    // 2d. Concurrency suite (c subset).
    for (const c of buildConcurrencyCases(providers)) {
      if (!accept(c)) continue;
      if (c.provider && downProviders.has(c.provider)) { recordSkip(c); continue; }
      record(await this.runCase(c));
    }

    // 3. Object-event suite (per-provider). Skip providers whose RPC probe
    // failed (downProviders) or whose observers were refused or didn't echo
    // a probe event (eventlessProviders). The local provider always probes
    // successfully because native-local dispatch is in-process.
    await this.probeEventPaths(eventProviders.filter((p) => !downProviders.has(p)));
    for (const c of buildEventCases(eventProviders)) {
      if (!accept(c)) continue;
      if (c.provider && downProviders.has(c.provider)) { recordSkip(c); continue; }
      if (c.provider && this.eventlessProviders.has(c.provider)) {
        record({
          case: c.name,
          provider: c.provider,
          status: 'ERROR',
          error: `provider ${c.provider} object-event path unreachable (observe or probe failed)`,
          duration_ms: 0,
        });
        continue;
      }
      record(await this.runCase(c));
    }

    // 4. Negative-path suite (runs against local).
    for (const c of ERROR_CASES) {
      if (!accept(c)) continue;
      record(await this.runCase(c));
    }

    // 4b. Ranged reads, then bucket management. Bucket edits rewrite the
    // worker's configuration (each reload keeps the untouched buckets and
    // their notification pollers), so they run after every other suite.
    for (const c of [...buildRangeCases(providers), ...buildBucketCases(providers)]) {
      if (!accept(c)) continue;
      if (c.provider && downProviders.has(c.provider)) { recordSkip(c); continue; }
      record(await this.runCase(c));
    }

    // 5. Empty-filter regression guard.
    const counted = results.filter((r) => r.case !== 'schema-reset');
    if (counted.length === 0) {
      record({
        case: this.opts.filterCase
          ? `filter "${this.opts.filterCase}" matched no cases`
          : 'no cases registered',
        status: 'FAIL',
        error: 'zero cases ran',
        duration_ms: 0,
      });
    }

    await this.disposeObservers();
    return this.persist({ results, uncountedCases: ['schema-reset'] });
  }

  private persist(args: { results: CaseResult[]; uncountedCases: string[] }): {
    pass: number; fail: number; errored: number; total: number; results: CaseResult[];
  } {
    const counted = args.results.filter((r) => !args.uncountedCases.includes(r.case));
    const pass = counted.filter((r) => r.status === 'PASS').length;
    const fail = counted.filter((r) => r.status === 'FAIL').length;
    const errored = counted.filter((r) => r.status === 'ERROR').length;

    const byProvider: Record<string, { pass: number; fail: number; error: number }> = {};
    for (const r of counted) {
      const key = r.provider ?? 'unscoped';
      byProvider[key] ??= { pass: 0, fail: 0, error: 0 };
      if (r.status === 'PASS') byProvider[key].pass++;
      else if (r.status === 'FAIL') byProvider[key].fail++;
      else byProvider[key].error++;
    }

    mkdirSync(resolve(this.opts.reportPath, '..'), { recursive: true });
    writeFileSync(
      this.opts.reportPath,
      JSON.stringify(
        {
          summary: { pass, fail, error: errored, total: counted.length },
          by_provider: byProvider,
          results: args.results,
        },
        null, 2,
      ),
    );
    return { pass, fail, errored, total: counted.length, results: args.results };
  }
}
