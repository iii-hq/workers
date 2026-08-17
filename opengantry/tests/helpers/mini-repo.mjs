import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);

function resolveMissionSchemaSrc() {
  const bundled = path.resolve(import.meta.dirname, '../fixtures/MISSION.schema.yaml');
  if (fs.existsSync(bundled)) return bundled;
  try {
    const pkgDir = path.dirname(require.resolve('@jeger-ai/opengantry/package.json'));
    const fromPkg = path.join(pkgDir, 'templates/.gitagent/planner/MISSION.schema.yaml');
    if (fs.existsSync(fromPkg)) return fromPkg;
  } catch {
    /* optional package */
  }
  throw new Error('MISSION.schema.yaml not found — missing tests/fixtures/MISSION.schema.yaml');
}

export function writeMiniGantryRepo(
  dir,
  { msnId = 'MSN-0175', missionRel = '.gitagent/missions/MSN-0175.yaml' } = {},
) {
  const schemaSrc = resolveMissionSchemaSrc();
  fs.mkdirSync(path.join(dir, '.gitagent', 'planner'), { recursive: true });
  fs.copyFileSync(schemaSrc, path.join(dir, '.gitagent', 'planner', 'MISSION.schema.yaml'));
  fs.mkdirSync(path.join(dir, '.gitagent', 'foreman'), { recursive: true });
  fs.mkdirSync(path.join(dir, '.gitagent', 'missions'), { recursive: true });
  fs.writeFileSync(
    path.join(dir, '.gitagent', 'foreman', 'MANIFEST.json'),
    JSON.stringify({
      schema_version: '0.5.0',
      skills: {
        gantry: {
          desc: 't',
          trust_threshold: 'Tier-2',
          tmvc_roots: ['src/'],
          forbidden_zones: [],
          gate_commands: ['npm test'],
        },
      },
      path_risks: {},
      risk_keywords: [],
      perimeter_protected: [],
    }),
  );
  fs.writeFileSync(
    path.join(dir, missionRel),
    `msn_id: ${msnId}
skill_key: gantry
gate_command: npm test
trace_rows: []
`,
  );
  fs.writeFileSync(
    path.join(dir, '.gitagent', 'foreman', 'ORG.export.local'),
    JSON.stringify({ org_id: 'demo-org', pepper: 'demo-pepper' }),
    { mode: 0o600 },
  );
  return { missionRel, msnId };
}

export function writeKeyring(dir) {
  const keyring = path.join(dir, 'pepper-keyring.json');
  fs.writeFileSync(
    keyring,
    JSON.stringify([{ org_id: 'demo-org', pepper_version: 1, pepper: 'demo-pepper' }]),
    { mode: 0o600 },
  );
  return keyring;
}
