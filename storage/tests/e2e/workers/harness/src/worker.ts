// Entry point: connect to the engine, register two sink functions for
// object-created / object-deleted dispatches, run the suite, emit the
// HARNESS_DONE sentinel that run-tests.sh greps for, then drain + shutdown.
//
// The harness is NOT registered in the engine's worker manifest — it runs
// as a plain host node process and connects to the engine over WebSocket
// like any external client. Only storage runs as an engine-managed
// worker; the harness is purely a client driving its RPCs.
import { registerWorker, Logger } from 'iii-sdk';
import { resolve } from 'node:path';
import { Runner } from './runner.ts';
import { ALL_PROVIDERS, type Provider } from './cases.ts';

const URL = process.env.III_URL ?? 'ws://127.0.0.1:49134';
const REPORT_PATH = resolve(process.env.HARNESS_REPORT_PATH ?? './reports/report.json');
const FILTER = process.env.HARNESS_FILTER;

// Validate against the canonical Provider list rather than blindly casting.
// A typo like `HARNESS_PROVIDERS=loval` would otherwise propagate silently
// and surface as a confusing "no bucket configured" error mid-test.
function parseProviders(raw: string, varName: string): Provider[] {
  const tokens = raw.split(',').map((s) => s.trim()).filter(Boolean);
  const valid = new Set<string>(ALL_PROVIDERS);
  const invalid = tokens.filter((t) => !valid.has(t));
  if (invalid.length > 0) {
    throw new Error(
      `${varName} contains unknown provider(s): ${invalid.join(', ')}. ` +
        `Valid: ${ALL_PROVIDERS.join(', ')}`,
    );
  }
  return tokens as Provider[];
}

const PROVIDERS_RAW = process.env.HARNESS_PROVIDERS ?? 'local';
const PROVIDERS = parseProviders(PROVIDERS_RAW, 'HARNESS_PROVIDERS');
// Optional subset of providers whose trigger plumbing is wired in this
// harness deployment. Defaults to PROVIDERS (fan triggers across every
// selected provider) so existing callers don't need to know about it.
// CI sets HARNESS_TRIGGER_PROVIDERS=local,s3 to keep r2 trigger ERRORs
// (no Cloudflare Queue plumbing in this harness) from failing the run.
const TRIGGER_PROVIDERS_RAW = process.env.HARNESS_TRIGGER_PROVIDERS;
const TRIGGER_PROVIDERS: Provider[] | undefined = TRIGGER_PROVIDERS_RAW
  ? parseProviders(TRIGGER_PROVIDERS_RAW, 'HARNESS_TRIGGER_PROVIDERS')
  : undefined;

const iii = registerWorker(URL);
const logger = new Logger();
const runner = new Runner({
  iii,
  reportPath: REPORT_PATH,
  filterCase: FILTER,
  providers: PROVIDERS,
  triggerProviders: TRIGGER_PROVIDERS,
});

iii.registerFunction(
  'harness::on_object_created',
  async (payload: unknown) => runner.onObjectCreated(payload),
  { description: 'Sink for storage::object-created dispatches.' },
);
iii.registerFunction(
  'harness::on_object_deleted',
  async (payload: unknown) => runner.onObjectDeleted(payload),
  { description: 'Sink for storage::object-deleted dispatches.' },
);

logger.info('harness: registered, kicking off suite', {
  url: URL,
  filter: FILTER ?? 'all',
  reportPath: REPORT_PATH,
});

(async () => {
  // ANSI colors only when stdout is a TTY — run-tests.sh redirects to a log file
  // and bash's grep for the HARNESS_DONE sentinel must see plain text.
  const useColor = process.stdout.isTTY === true;
  const GREEN = useColor ? '\x1b[32m' : '';
  const RED = useColor ? '\x1b[31m' : '';
  const RESET = useColor ? '\x1b[0m' : '';
  let exitCode = 1;
  try {
    const { pass, fail, errored, total } = await runner.runAll();
    let status: 'PASS' | 'FAIL' | 'ERROR';
    if (fail > 0) status = 'FAIL';
    else if (errored > 0) status = 'ERROR';
    else status = 'PASS';
    const color = status === 'PASS' ? GREEN : RED;
    const errorTail = errored > 0 ? ` errors=${errored}` : '';
    console.log(`HARNESS_DONE: ${color}${status}${RESET} ${pass}/${total}${errorTail}`);
    exitCode = status === 'PASS' ? 0 : status === 'ERROR' ? 2 : 1;
  } catch (e: any) {
    console.error('[harness] fatal:', e?.stack ?? e);
    console.log(`HARNESS_DONE: ${RED}FAIL${RESET} 0/0`);
    exitCode = 1;
  }
  // runAll() called runner.unregisterAllTriggers() which writes UnregisterTrigger
  // messages to the websocket synchronously. The SDK's Trigger.unregister() is
  // fire-and-forget — sendMessage queues bytes but doesn't await the engine ACK.
  // Without this drain step, process.exit() terminates before the OS flushes
  // the TCP send buffer, the storage worker never sees the unregister, and its
  // trigger registry keeps a zombie subscriber wired to the harness function id
  // that's about to disappear — causing the engine to log "Function not found:
  // harness::on_object_*" on every subsequent dispatch until the next run
  // re-registers a fresh subscription.
  //
  // 200ms grace lets the OS flush ws bytes; iii.shutdown() then closes the ws
  // and drains OTel queues. iii.shutdown() itself does NOT await the ws close
  // handshake, hence the explicit delay.
  await new Promise((r) => setTimeout(r, 200));
  await iii.shutdown();
  process.exit(exitCode);
})();
