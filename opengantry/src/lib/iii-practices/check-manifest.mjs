import fs from 'node:fs';
import path from 'node:path';
import { listWorkerRoots, walkFiles } from './scan-workers.mjs';

const DEPLOY_MODES = new Set(['binary', 'image', 'bundle']);

function yamlScalar(text, key) {
  const m = text.match(new RegExp(`^${key}:\\s*(\\S+)`, 'm'));
  return m ? m[1].replace(/^["']|["']$/g, '') : null;
}

function hasTags(text) {
  if (/^tags:\s*\[\s*\]/m.test(text)) return false;
  if (/^tags:\s*\n\s+-\s*\S/m.test(text)) return true;
  if (/^tags:\s*\[\s*['"]?\S/m.test(text)) return true;
  if (/^tags:\s*$/m.test(text)) return false;
  return false;
}

function hasScriptsStart(text) {
  return (
    /(?:^|\n)scripts:\s*\n(?:.*\n)*?\s+start:\s*\S/m.test(text) ||
    /(?:^|\n)scripts:\s*\n\s+start:\s*\S/m.test(text)
  );
}

export function checkWorkerManifest(scanRoot) {
  const findings = [];
  for (const workerDir of listWorkerRoots(scanRoot)) {
    const folder = path.basename(workerDir);
    const yamlPath = path.join(workerDir, 'iii.worker.yaml');
    const relYaml = path.join(folder, 'iii.worker.yaml');
    if (!fs.existsSync(yamlPath)) {
      findings.push({
        rule_id: 'manifest/missing',
        file: relYaml,
        line: 1,
        message: 'iii.worker.yaml required',
      });
      continue;
    }
    const text = fs.readFileSync(yamlPath, 'utf8');
    const name = yamlScalar(text, 'name');
    if (!name) {
      findings.push({
        rule_id: 'manifest/name',
        file: relYaml,
        line: 1,
        message: 'iii.worker.yaml must set name',
      });
    } else if (name !== folder) {
      findings.push({
        rule_id: 'manifest/name',
        file: relYaml,
        line: 1,
        message: `name ${name} must equal folder ${folder}`,
      });
    }
    if (!yamlScalar(text, 'language')) {
      findings.push({
        rule_id: 'manifest/language',
        file: relYaml,
        line: 1,
        message: 'iii.worker.yaml must set language',
      });
    }
    const deploy = yamlScalar(text, 'deploy');
    if (!DEPLOY_MODES.has(deploy)) {
      findings.push({
        rule_id: 'manifest/deploy',
        file: relYaml,
        line: 1,
        message: 'deploy must be binary, image, or bundle',
      });
    }
    if (!hasTags(text)) {
      findings.push({
        rule_id: 'manifest/tags',
        file: relYaml,
        line: 1,
        message: 'iii.worker.yaml must set a non-empty tags list',
      });
    }
    if (!hasScriptsStart(text)) {
      findings.push({
        rule_id: 'manifest/scripts-start',
        file: relYaml,
        line: 1,
        message: 'iii.worker.yaml must set scripts.start',
      });
    }
    if (deploy === 'bundle') {
      if (/^\s+setup:/m.test(text)) {
        findings.push({
          rule_id: 'manifest/bundle-setup',
          file: relYaml,
          line: 1,
          message: 'bundle workers must not set scripts.setup',
        });
      }
      if (/^\s+install:/m.test(text)) {
        findings.push({
          rule_id: 'manifest/bundle-install',
          file: relYaml,
          line: 1,
          message: 'bundle workers must not set scripts.install',
        });
      }
      if (/base_image:/.test(text)) {
        findings.push({
          rule_id: 'manifest/bundle-base-image',
          file: relYaml,
          line: 1,
          message: 'bundle workers must not set runtime.base_image',
        });
      }
    }

    const skillPath = path.join(workerDir, 'skills', 'SKILL.md');
    if (!fs.existsSync(skillPath)) {
      findings.push({
        rule_id: 'layout/skill',
        file: path.join(folder, 'skills/SKILL.md'),
        line: 1,
        message: 'skills/SKILL.md required',
      });
    }
    const testsDir = path.join(workerDir, 'tests');
    const testFiles = fs.existsSync(testsDir) ? walkFiles(testsDir) : [];
    let hasTestScript = false;
    try {
      const pkg = JSON.parse(fs.readFileSync(path.join(workerDir, 'package.json'), 'utf8'));
      hasTestScript = typeof pkg.scripts?.test === 'string' && pkg.scripts.test.trim().length > 0;
    } catch {
      hasTestScript = false;
    }
    if (testFiles.length === 0 && !hasTestScript) {
      findings.push({
        rule_id: 'layout/tests',
        file: path.join(folder, 'tests'),
        line: 1,
        message: 'tests/ must be non-empty, or package.json scripts.test must exist',
      });
    }
  }
  return findings;
}
