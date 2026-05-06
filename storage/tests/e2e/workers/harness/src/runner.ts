import { writeFileSync, mkdirSync } from 'node:fs';
import { resolve } from 'node:path';
import type { ISdk } from 'iii-sdk';
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
import { buildTriggerCases } from './cases-triggers.ts';
import { ERROR_CASES } from './cases-errors.ts';
import {
  buildBodyShapeCases,
  buildKeyShapeCases,
  buildMetadataCases,
  buildErrorEnvelopeCases,
} from './cases-edge.ts';
import { buildProviderQuirkCases } from './cases-provider.ts';
import { buildConcurrencyCases } from './cases-concurrency.ts';

interface CaseResult {
  case: string;
  provider?: Provider;
  status: 'PASS' | 'FAIL' | 'ERROR';
  error?: string;
  duration_ms: number;
}

export interface RunnerOptions {
  iii: ISdk;
  reportPath: string;
  providers: Provider[];
  /** Optional case-name substring filter — when set, only matching cases run. */
  filterCase?: string;
}

const READY_TIMEOUT_MS = 30_000;
const READY_PROBE_INTERVAL_MS = 250;

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

  private triggers: { unregister: () => void }[] = [];

  /**
   * Providers whose trigger plumbing didn't deliver a probe event within
   * the deadline. Their trigger cases get ERROR-skipped instead of FAILing
   * so a half-broken cloud profile (e.g., MinIO up, bridge down) doesn't
   * mask unrelated regressions in the rest of the suite.
   */
  private triggerlessProviders = new Set<Provider>();

  constructor(private opts: RunnerOptions) {}

  // ---------------- sink fns ----------------

  /**
   * The `storage::object-created` trigger payload shape (per
   * storage/src/triggers/object_created.rs::Event):
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
   * The `storage::object-deleted` trigger payload shape (per
   * storage/src/triggers/object_deleted.rs::Event):
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
    return await this.opts.iii.trigger<unknown, any>({ function_id: functionId, payload });
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
    // Cancel any pending waiters first; their cases either already failed
    // or are no longer interested.
    for (const w of this.waiters) clearTimeout(w.timer);
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
    // `Function not found` is the most common timeout cause: the storage
    // worker connected to the engine but exited before registering its RPCs
    // — almost always because rustfs is missing (see the worker's
    // LOCAL_BACKEND_BIN_NOT_FOUND error). Surface a hint instead of just
    // dumping the raw error so users don't have to grep `iii worker logs`.
    const lastMsg = (lastErr as any)?.message ?? String(lastErr);
    const hint = lastMsg.includes('Function not found')
      ? '\n  hint: storage may have exited before registering RPCs. Run `iii worker logs storage` to inspect; LOCAL_BACKEND_BIN_NOT_FOUND means rustfs is missing — set $RUSTFS_BIN or install rustfs on $PATH.'
      : '';
    throw new Error(
      `storage worker did not become ready within ${READY_TIMEOUT_MS}ms; last error: ${lastMsg}${hint}`,
    );
  }

  // ---------------- trigger lifecycle ----------------

  private registerTriggers(providers: readonly Provider[]): void {
    // Register both trigger types pointing at our two sink functions, once
    // per provider. The 60s handler_timeout is the storage default (see
    // triggers/object_created.rs::default_handler_timeout); we leave it
    // as-is since our handlers are near-instant.
    //
    // If a provider's bucket isn't wired for triggers in the worker config
    // (e.g., scratch-s3 missing the `notifications.sqs_queue_url` field),
    // registerTrigger throws synchronously per
    // storage/src/triggers/handler.rs::register, and we tag the provider
    // as triggerless so its cases ERROR-skip later.
    for (const provider of providers) {
      const bucket = BUCKETS[provider];
      try {
        this.triggers.push(
          this.opts.iii.registerTrigger({
            type: 'storage::object-created',
            function_id: 'harness::on_object_created',
            config: { bucket },
          }),
          this.opts.iii.registerTrigger({
            type: 'storage::object-deleted',
            function_id: 'harness::on_object_deleted',
            config: { bucket },
          }),
        );
      } catch (e: unknown) {
        this.triggerlessProviders.add(provider);
        const msg = e instanceof Error ? e.message : String(e);
        console.error(`[harness] trigger registration failed for ${provider}: ${msg}`);
      }
    }
  }

  /**
   * Smoke-test the trigger delivery path for each provider that managed to
   * register triggers. Local skips the probe (in-process webhook is
   * deterministic). Cloud providers do a put + waitForEvent and on timeout
   * are added to `triggerlessProviders`. The probe key namespace is
   * disjoint from scenario keys so any leftover events drained at the
   * start of the first scenario via `flush()` won't confuse assertions.
   */
  private async probeTriggerPaths(providers: readonly Provider[]): Promise<void> {
    const PROBE_TIMEOUT_MS = 8_000;
    for (const provider of providers) {
      if (provider === 'local') continue;
      if (this.triggerlessProviders.has(provider)) continue;
      const bucket = BUCKETS[provider];
      const key = `harness/.trigger-probe-${provider}-${Date.now()}`;
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
        this.triggerlessProviders.add(provider);
        const msg = e instanceof Error ? e.message : String(e);
        console.error(`[harness] trigger probe failed for ${provider}: ${msg}`);
      }
    }
  }

  /**
   * Unregister all active triggers. Without this, re-running the harness
   * against the same long-running worker leaves zombie subscribers in the
   * trigger registry and the next run double-counts events. The 200ms drain
   * in worker.ts before process.exit() is what gives these unregister
   * messages time to actually leave the WebSocket buffer.
   */
  private unregisterAllTriggers(): void {
    for (const t of this.triggers) {
      try {
        t.unregister();
      } catch (e) {
        console.error(`[harness] unregister trigger failed: ${e}`);
      }
    }
    this.triggers = [];
  }

  // ---------------- public entry ----------------

  async runAll(): Promise<{ pass: number; fail: number; errored: number; total: number; results: CaseResult[] }> {
    await this.waitForStorageWorker();
    this.registerTriggers(this.opts.providers);

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
      this.unregisterAllTriggers();
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

    // 3. Trigger dispatch suite (per-provider). Skip providers whose RPC
    // probe failed (downProviders) or whose trigger plumbing didn't echo
    // a probe event (triggerlessProviders). The local provider always
    // probes successfully because rustfs's webhook is in-process.
    await this.probeTriggerPaths(providers.filter((p) => !downProviders.has(p)));
    for (const c of buildTriggerCases(providers)) {
      if (!accept(c)) continue;
      if (c.provider && downProviders.has(c.provider)) { recordSkip(c); continue; }
      if (c.provider && this.triggerlessProviders.has(c.provider)) {
        record({
          case: c.name,
          provider: c.provider,
          status: 'ERROR',
          error: `provider ${c.provider} trigger path unreachable (probe failed)`,
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

    this.unregisterAllTriggers();
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
