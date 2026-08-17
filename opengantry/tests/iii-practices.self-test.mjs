import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { fileURLToPath } from 'node:url';
import { scanWorkersTree } from '../src/lib/iii-practices/scan.mjs';
import {
  loadHttpConnectorAllowlist,
  resolveRepoRoot,
} from '../src/lib/iii-practices/allowlist.mjs';

function materializeFixture(fixtureName) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), `iii-arch-${fixtureName}-`));
  const workers = path.join(dir, 'workers');
  fs.mkdirSync(workers, { recursive: true });
  return {
    dir,
    workers,
    write: (rel, content) => {
      const p = path.join(workers, rel);
      fs.mkdirSync(path.dirname(p), { recursive: true });
      fs.writeFileSync(p, content);
    },
  };
}

function writeManifest(fx, name, extraYaml = '') {
  fx.write(`${name}/package.json`, JSON.stringify({ name, private: true, type: 'module' }));
  fx.write(
    `${name}/iii.worker.yaml`,
    `iii: v1
name: ${name}
language: javascript
deploy: bundle
manifest: package.json
tags:
  - test
scripts:
  start: node ./index.mjs
${extraYaml}`,
  );
  fx.write(`${name}/skills/SKILL.md`, `# ${name}\n`);
  fx.write(`${name}/tests/smoke.test.js`, 'export const ok = 1;\n');
}

export async function runSelfTest(scanOpts) {
  const repoRoot = resolveRepoRoot();
  const opts = scanOpts ?? {
    httpAllowlist: loadHttpConnectorAllowlist(repoRoot).workers,
  };
  const cases = [];

  {
    const fx = materializeFixture('fetch');
    fx.write('evil/package.json', JSON.stringify({ name: 'evil', private: true, type: 'module' }));
    fx.write('evil/src/index.js', "export async function go() { await fetch('https://x'); }\n");
    const { findings } = await scanWorkersTree(fx.workers, opts);
    cases.push({
      name: 'bad-fetch',
      ok: findings.some((f) => f.rule_id.startsWith('async/')),
    });
  }

  {
    const fx = materializeFixture('pragma-denied');
    fx.write(
      'pragma-worker/package.json',
      JSON.stringify({ name: 'pragma-worker', private: true, type: 'module' }),
    );
    fx.write(
      'pragma-worker/src/index.js',
      `/* gantry-allow-external-http */
export async function go() { await fetch('https://x'); }
`,
    );
    const { findings } = await scanWorkersTree(fx.workers, opts);
    cases.push({
      name: 'pragma-without-allowlist',
      ok: findings.some((f) => f.rule_id === 'async/http-pragma-denied'),
    });
  }

  {
    const fx = materializeFixture('pragma-allowed');
    const allowPath = path.join(fx.dir, 'allowlist.json');
    fs.writeFileSync(allowPath, JSON.stringify({ http_connector_workers: ['connector'] }));
    const prev = process.env.GANTRY_III_ARCH_ALLOWLIST;
    process.env.GANTRY_III_ARCH_ALLOWLIST = allowPath;
    try {
      fx.write(
        'connector/package.json',
        JSON.stringify({ name: 'connector', private: true, type: 'module' }),
      );
      fx.write(
        'connector/src/index.js',
        `/* gantry-allow-external-http */
export async function go() { await fetch('https://x'); }
`,
      );
      const { workers: tempAllow } = loadHttpConnectorAllowlist(repoRoot);
      const { findings } = await scanWorkersTree(fx.workers, { httpAllowlist: tempAllow });
      cases.push({
        name: 'pragma-with-allowlist',
        ok: !findings.some((f) => f.rule_id.startsWith('async/http')),
      });
    } finally {
      if (prev === undefined) delete process.env.GANTRY_III_ARCH_ALLOWLIST;
      else process.env.GANTRY_III_ARCH_ALLOWLIST = prev;
    }
  }

  {
    const fx = materializeFixture('nopkg');
    fx.write('orphan/src/index.js', 'export const x = 1;\n');
    const { findings } = await scanWorkersTree(fx.workers, opts);
    cases.push({
      name: 'missing-package-json',
      ok: findings.some((f) => f.rule_id === 'worker/package-json'),
    });
  }

  {
    const fx = materializeFixture('import-id');
    writeManifest(fx, 'w');
    fx.write('w/src/ids.js', "export const ID = 'a::b';\n");
    fx.write(
      'w/src/index.js',
      "import { ID } from './ids.js';\nworker.registerFunction(ID, async () => ({}));\n",
    );
    const { findings } = await scanWorkersTree(fx.workers, opts);
    cases.push({
      name: 'imported-register-id',
      ok: findings.some((f) => f.rule_id === 'payload/register-id'),
    });
  }

  {
    const fx = materializeFixture('ts-allowed');
    writeManifest(fx, 'typed');
    fx.write(
      'typed/src/index.ts',
      `const worker = { registerFunction(_id: string, _fn: unknown, _opts: object) { return; } };
worker.registerFunction("typed::ping", async () => ({ ok: true as const }), {
  request_format: { type: "object", properties: { n: { type: "number" } } },
  response_format: { type: "object", properties: { ok: { type: "boolean" } } },
});
`,
    );
    const { findings } = await scanWorkersTree(fx.workers, opts);
    cases.push({
      name: 'ts-extension',
      ok:
        !findings.some((f) => f.rule_id === 'worker/js-only') &&
        !findings.some((f) => f.rule_id === 'payload/parse') &&
        !findings.some((f) => f.rule_id === 'payload/request-response-format'),
    });
  }

  {
    const fx = materializeFixture('missing-formats');
    writeManifest(fx, 'plain');
    fx.write(
      'plain/src/index.js',
      'worker.registerFunction("plain::ping", async () => ({ ok: true }));\n',
    );
    const { findings } = await scanWorkersTree(fx.workers, opts);
    cases.push({
      name: 'missing-formats',
      ok: findings.some((f) => f.rule_id === 'payload/request-response-format'),
    });
  }

  {
    const fx = materializeFixture('bundle-yaml');
    fx.write(
      'bundled/package.json',
      JSON.stringify({ name: 'bundled', private: true, type: 'module' }),
    );
    fx.write(
      'bundled/iii.worker.yaml',
      `iii: v1
name: bundled
language: javascript
deploy: bundle
manifest: package.json
tags:
  - test
runtime:
  base_image: docker.io/iiidev/node:latest
scripts:
  install: npm install
  start: node ./index.mjs
`,
    );
    fx.write('bundled/skills/SKILL.md', '# bundled\n');
    fx.write('bundled/tests/smoke.test.js', 'export const ok = 1;\n');
    fx.write('bundled/src/index.js', 'export const ok = 1;\n');
    const { findings } = await scanWorkersTree(fx.workers, opts);
    cases.push({
      name: 'bundle-yaml',
      ok:
        findings.some((f) => f.rule_id === 'manifest/bundle-install') &&
        findings.some((f) => f.rule_id === 'manifest/bundle-base-image'),
    });
  }

  {
    const fx = materializeFixture('const-bag');
    fx.write('w/package.json', JSON.stringify({ name: 'w', private: true, type: 'module' }));
    fx.write('w/src/index.js', 'const state = { cache: new Map() };\nexport const ok = state;\n');
    const { findings } = await scanWorkersTree(fx.workers, opts);
    cases.push({
      name: 'const-module-bag',
      ok: findings.some((f) => f.rule_id === 'durable-state/module-bags'),
    });
  }

  {
    const fx = materializeFixture('global');
    fx.write('w/package.json', JSON.stringify({ name: 'w', private: true, type: 'module' }));
    fx.write('w/src/index.js', 'global.orchestrationState = {};\n');
    const { findings } = await scanWorkersTree(fx.workers, opts);
    cases.push({
      name: 'global-assign',
      ok: findings.some((f) => f.rule_id === 'durable-state/global-process'),
    });
  }

  {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'iii-arch-single-root-'));
    fs.writeFileSync(
      path.join(dir, 'package.json'),
      JSON.stringify({ name: 'solo', private: true, type: 'module' }),
    );
    fs.mkdirSync(path.join(dir, 'src'), { recursive: true });
    fs.writeFileSync(path.join(dir, 'src', 'index.js'), 'export const ok = 1;\n');
    const { findings } = await scanWorkersTree(dir, opts);
    cases.push({
      name: 'single-worker-scan-root',
      ok: !findings.some((f) => f.rule_id === 'worker/package-json'),
    });
  }

  {
    const fx = materializeFixture('compliant');
    writeManifest(fx, 'good');
    fx.write(
      'good/schemas/good__ping.json',
      JSON.stringify({
        $schema: 'http://json-schema.org/draft-07/schema#',
        type: ['object', 'string', 'number', 'boolean', 'null'],
      }),
    );
    fx.write(
      'good/src/index.js',
      `worker.registerFunction('good::ping', async () => ({ ok: true }), {
  request_format: loadSchema('good__ping.json'),
  response_format: loadSchema('good__ping.json'),
});
`,
    );
    const { findings } = await scanWorkersTree(fx.workers, opts);
    cases.push({
      name: 'compliant-worker',
      ok: findings.length === 0,
    });
  }

  {
    const fx = materializeFixture('path-traversal');
    writeManifest(fx, 'leaky');
    fx.write(
      'leaky/src/index.js',
      "import fs from 'node:fs';\nfs.writeFileSync('.gitagent/../../tmp/evil', 'x');\n",
    );
    const { findings } = await scanWorkersTree(fx.workers, opts);
    cases.push({
      name: 'path-traversal-write',
      ok: findings.some((f) => f.rule_id === 'durable-state/fs-writes'),
    });
  }

  {
    const fx = materializeFixture('trigger-type');
    writeManifest(fx, 'bad-trigger');
    fx.write('bad-trigger/src/index.js', "worker.registerTriggerType({ id: 'x::y' });\n");
    const { findings } = await scanWorkersTree(fx.workers, opts);
    cases.push({
      name: 'register-trigger-type-arity',
      ok: findings.some((f) => f.rule_id === 'runtime/register-trigger-type'),
    });
  }

  {
    const fx = materializeFixture('empty-tags');
    fx.write(
      'notags/package.json',
      JSON.stringify({ name: 'notags', private: true, type: 'module' }),
    );
    fx.write(
      'notags/iii.worker.yaml',
      `iii: v1
name: notags
language: javascript
deploy: bundle
manifest: package.json
tags: []
scripts:
  start: node ./index.mjs
`,
    );
    fx.write('notags/skills/SKILL.md', '# notags\n');
    fx.write('notags/tests/smoke.test.js', 'export const ok = 1;\n');
    const { findings } = await scanWorkersTree(fx.workers, opts);
    cases.push({
      name: 'empty-tags',
      ok: findings.some((f) => f.rule_id === 'manifest/tags'),
    });
  }

  let failed = 0;
  for (const c of cases) {
    if (!c.ok) {
      console.error(`self-test FAIL: ${c.name}`);
      failed += 1;
    } else {
      console.log(`self-test PASS: ${c.name}`);
    }
  }
  if (failed) {
    console.error(
      'iii-architecture: EXIT 1 — architecture / code violations found (self-test expectations)',
    );
    process.exit(1);
  }
}

const launchedDirectly =
  Boolean(process.argv[1]) && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (launchedDirectly) {
  await runSelfTest();
}
