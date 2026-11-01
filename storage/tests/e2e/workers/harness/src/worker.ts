// Entry point: connect to the engine, register two sink functions that
// observe storage::on-object-created / storage::on-object-deleted, run the
// suite, emit the HARNESS_DONE sentinel that run-tests.sh greps for, then
// shutdown.
//
// The harness is NOT registered in the engine's worker manifest — it runs
// as a plain host node process and connects to the engine over WebSocket
// like any external client. Only storage runs as an engine-managed
// worker; the harness is purely a client driving its RPCs.
import { registerWorker } from 'iii-sdk';
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
// Optional subset of providers whose object-event plumbing is wired in
// this harness deployment. Defaults to PROVIDERS (fan object events across
// every selected provider) so existing callers don't need to know about it.
// CI sets HARNESS_EVENT_PROVIDERS=local,s3 to keep r2 object-event ERRORs
// (no Cloudflare Queue plumbing in this harness) from failing the run.
const EVENT_PROVIDERS_RAW = process.env.HARNESS_EVENT_PROVIDERS;
const EVENT_PROVIDERS: Provider[] | undefined = EVENT_PROVIDERS_RAW
  ? parseProviders(EVENT_PROVIDERS_RAW, 'HARNESS_EVENT_PROVIDERS')
  : undefined;

const iii = registerWorker(URL);
const runner = new Runner({
  iii,
  reportPath: REPORT_PATH,
  filterCase: FILTER,
  providers: PROVIDERS,
  eventProviders: EVENT_PROVIDERS,
});

iii.registerFunction(
  'harness::on_object_created',
  async (payload: unknown) => runner.onObjectCreated(payload),
  { description: 'Observer target for storage::on-object-created events.' },
);
iii.registerFunction(
  'harness::on_object_deleted',
  async (payload: unknown) => runner.onObjectDeleted(payload),
  { description: 'Observer target for storage::on-object-deleted events.' },
);

console.info('harness: registered, kicking off suite', {
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
  // A completed runAll() disposed every observer it created and awaited the
  // engine's confirmation, so nothing still targets the sink functions that
  // disappear with this process. iii.shutdown() closes the ws and drains
  // OTel queues.
  await iii.shutdown();
  process.exit(exitCode);
})();
