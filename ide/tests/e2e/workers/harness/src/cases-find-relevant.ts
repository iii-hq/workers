import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { expect, expectEqual, type CaseContext, type TestCase } from './cases.ts';

// coder::find-relevant over the real engine with a fake `judge::evaluate`
// registered by this harness (the cases-fs-sandbox.ts mock precedent): the
// fake records every payload the ide worker sends and answers 0.9 for a
// question whose item (or, without items, whose state) names the marker
// file, 0.05 otherwise.

const MARKER = 'judge_marker';
const DEADLINE_MS = 15_000;

const FILES: Record<string, string> = {
  [`src/${MARKER}.rs`]: `pub fn ${MARKER}() -> u32 {\n    42\n}\n`,
  'src/other.rs': 'pub fn other() {}\n',
  'noise/a/b/c/leaf.txt': 'NOISE_LEAF\n',
  '.env': 'SECRET_ENV=1\n',
  id_rsa: 'SECRET_RSA\n',
  // Split so push-time secret scanners do not flag the fixture.
  'deploy_material.txt': '-----BEGIN OPENSSH PRIVATE' + ' KEY-----\nSECRET_PK\n',
};
// Nothing of these may reach the judge, by name or by content.
const FORBIDDEN = ['.env', 'id_rsa', 'deploy_material', 'SECRET_', 'noise/a/b', 'NOISE_LEAF'];

function fixture(): string {
  const root = mkdtempSync(join(tmpdir(), 'iii-ide-find-relevant-'));
  for (const [path, text] of Object.entries(FILES)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), text);
  }
  return root;
}

/// 0.9 or 0.05 for every `noul` key: a navigation key `q007` reads
/// `state.items[7]`, any other request reads its whole state. The ide sends
/// each state as JSON text so the engine cannot reorder its keys.
function answer(evaluation: any): Record<string, { type: 'noul'; noul: number }> {
  const state = typeof evaluation.state === 'string' ? JSON.parse(evaluation.state) : evaluation.state;
  const items: unknown[] | undefined = state?.items;
  const answers: Record<string, { type: 'noul'; noul: number }> = {};
  for (const key of Object.keys(evaluation.questions ?? {})) {
    const index = /^q(\d+)$/.exec(key);
    const subject = items && index ? items[Number(index[1])] : state;
    const hit = JSON.stringify(subject ?? '').includes(MARKER);
    answers[key] = { type: 'noul', noul: hit ? 0.9 : 0.05 };
  }
  return answers;
}

/// Poll the engine until `judge::evaluate` is (or is no longer) listed.
async function awaitJudge({ call, sleep }: CaseContext, present: boolean): Promise<void> {
  for (let i = 0; i < 50; i++) {
    const listed = await call('engine::functions::info', { function_id: 'judge::evaluate' }).then(
      (info) => Boolean(info) && info.found !== false,
      () => false,
    );
    if (listed === present) return;
    await sleep(100);
  }
  throw new Error(`judge::evaluate ${present ? 'never registered' : 'never unregistered'}`);
}

export const FIND_RELEVANT_CASES: TestCase[] = [
  {
    name: 'find_relevant_with_fake_judge_returns_marker_and_withholds_secrets',
    async run(ctx: CaseContext) {
      const { call, iii } = ctx;
      const root = fixture();
      const payloads: any[] = [];
      const fake = iii.registerFunction('judge::evaluate', async (payload: any) => {
        payloads.push(payload);
        const results: Record<string, unknown> = {};
        for (const evaluation of payload.evaluations ?? []) {
          results[evaluation.id] = { answers: answer(evaluation) };
        }
        return { status: 'ok', model: 'fake', results, stats: { input_tokens: 1 } };
      });
      try {
        await awaitJudge(ctx, true);
        const out = await call('coder::find-relevant', {
          query: `which function returns the answer in ${MARKER}?`,
          path: root,
          timeout_ms: DEADLINE_MS,
        });

        expectEqual(out.status, 'complete', `status (reason ${out.reason}, issues ${JSON.stringify(out.issues)})`);
        expectEqual(out.reason, null, 'reason');
        expect(Array.isArray(out.files) && out.files.length > 0, 'files is a non-empty array');
        for (const file of out.files) {
          expect(typeof file.path === 'string' && file.path.startsWith(root), `absolute path: ${file.path}`);
          expect(file.score >= 0 && file.score <= 1, `score in [0,1]: ${file.score}`);
          expect(Array.isArray(file.roles), 'roles array');
          expect(Array.isArray(file.excerpts), 'excerpts array');
          expect(Array.isArray(file.leads), 'leads array');
          expect(Array.isArray(file.call_leads), 'call_leads array');
          expect(typeof file.source_omitted === 'boolean', 'source_omitted bool');
        }
        const marker = out.files[0];
        expectEqual(marker.path, join(root, `src/${MARKER}.rs`), 'marker file ranks first');
        expect(
          marker.excerpts.some((e: any) => e.line_from >= 1 && e.line_to >= e.line_from && e.text.includes(`fn ${MARKER}`)),
          `marker excerpt: ${JSON.stringify(marker.excerpts)}`,
        );
        expect(Array.isArray(out.agents_md), 'agents_md array');
        expect(typeof out.issues === 'object' && out.issues !== null, 'issues object');
        for (const stat of ['judge_calls', 'questions', 'input_tokens', 'cache_hits', 'elapsed_ms']) {
          expect(Number.isInteger(out.stats?.[stat]), `stats.${stat} is an integer`);
        }
        expect(out.stats.judge_calls + out.stats.cache_hits > 0, 'the judge was asked');

        expect(payloads.length > 0, 'the fake judge was called');
        const sent = JSON.stringify(payloads);
        for (const payload of payloads) {
          expect(!('provider' in payload), 'no provider without session baggage');
          expect(payload.timeout_ms >= 1 && payload.timeout_ms <= DEADLINE_MS, `timeout_ms ${payload.timeout_ms}`);
        }
        expect(sent.includes(`src/${MARKER}.rs`), 'root-relative marker path sent');
        expect(!sent.includes(root), 'the absolute root never leaves the host');
        for (const forbidden of FORBIDDEN) {
          expect(!sent.includes(forbidden), `${forbidden} reached the judge`);
        }
      } finally {
        fake.unregister();
      }

      // Without a judge the ask is a typed result, not an error. A fresh
      // query misses the worker's answer cache.
      await awaitJudge(ctx, false);
      const gone = await call('coder::find-relevant', {
        query: 'where is the entry point?',
        path: root,
        timeout_ms: DEADLINE_MS,
      });
      expectEqual(gone.status, 'unavailable', `status without a judge (reason ${gone.reason})`);
      expectEqual(gone.reason, 'not registered', 'reason');
      expectEqual(gone.files, [], 'no files');
    },
  },
];
