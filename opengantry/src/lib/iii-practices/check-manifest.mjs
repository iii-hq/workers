import fs from 'node:fs';
import path from 'node:path';
import YAML from 'yaml';

import { listWorkerRoots, walkFiles } from './scan-workers.mjs';

const DEPLOY_MODES = new Set(['binary', 'image', 'bundle']);

function lineAt(doc, node) {
  if (node?.range?.[0] != null && doc.lineCounter) {
    return doc.lineCounter.linePos(node.range[0]).line;
  }
  return 1;
}

function scalar(doc, key) {
  const node = doc.get(key, true);
  if (!node || node.value == null) return null;
  return String(node.value);
}

function hasNonEmptyTags(doc) {
  const tags = doc.get('tags');
  if (!tags) return false;
  if (YAML.isSeq(tags)) return tags.items.length > 0;
  if (YAML.isScalar(tags) && Array.isArray(tags.value)) return tags.value.length > 0;
  return false;
}

function hasScriptsStart(doc) {
  const scripts = doc.get('scripts');
  if (!scripts || !YAML.isMap(scripts)) return false;
  const start = scripts.get('start', true);
  return Boolean(start?.value);
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
    const doc = YAML.parseDocument(text);
    if (doc.errors?.length) {
      findings.push({
        rule_id: 'manifest/parse',
        file: relYaml,
        line: 1,
        message: `invalid iii.worker.yaml: ${doc.errors[0].message}`,
      });
      continue;
    }

    const nameNode = doc.get('name', true);
    const name = nameNode?.value != null ? String(nameNode.value) : null;
    if (!name) {
      findings.push({
        rule_id: 'manifest/name',
        file: relYaml,
        line: lineAt(doc, nameNode),
        message: 'iii.worker.yaml must set name',
      });
    } else if (name !== folder) {
      findings.push({
        rule_id: 'manifest/name',
        file: relYaml,
        line: lineAt(doc, nameNode),
        message: `name ${name} must equal folder ${folder}`,
      });
    }

    const langNode = doc.get('language', true);
    if (!langNode?.value) {
      findings.push({
        rule_id: 'manifest/language',
        file: relYaml,
        line: lineAt(doc, langNode),
        message: 'iii.worker.yaml must set language',
      });
    }

    const deployNode = doc.get('deploy', true);
    const deploy = deployNode?.value != null ? String(deployNode.value) : null;
    if (!DEPLOY_MODES.has(deploy)) {
      findings.push({
        rule_id: 'manifest/deploy',
        file: relYaml,
        line: lineAt(doc, deployNode),
        message: 'deploy must be binary, image, or bundle',
      });
    }

    if (!hasNonEmptyTags(doc)) {
      const tagsNode = doc.get('tags', true);
      findings.push({
        rule_id: 'manifest/tags',
        file: relYaml,
        line: lineAt(doc, tagsNode),
        message: 'iii.worker.yaml must set a non-empty tags list',
      });
    }

    if (!hasScriptsStart(doc)) {
      const scriptsNode = doc.get('scripts', true);
      findings.push({
        rule_id: 'manifest/scripts-start',
        file: relYaml,
        line: lineAt(doc, scriptsNode),
        message: 'iii.worker.yaml must set scripts.start',
      });
    }

    if (deploy === 'bundle') {
      const scripts = doc.get('scripts');
      if (scripts && YAML.isMap(scripts)) {
        const setupNode = scripts.get('setup', true);
        if (setupNode) {
          findings.push({
            rule_id: 'manifest/bundle-setup',
            file: relYaml,
            line: lineAt(doc, setupNode),
            message: 'bundle workers must not set scripts.setup',
          });
        }
        const installNode = scripts.get('install', true);
        if (installNode) {
          findings.push({
            rule_id: 'manifest/bundle-install',
            file: relYaml,
            line: lineAt(doc, installNode),
            message: 'bundle workers must not set scripts.install',
          });
        }
      }
      const runtime = doc.get('runtime');
      if (runtime && YAML.isMap(runtime)) {
        const baseImageNode = runtime.get('base_image', true);
        if (baseImageNode) {
          findings.push({
            rule_id: 'manifest/bundle-base-image',
            file: relYaml,
            line: lineAt(doc, baseImageNode),
            message: 'bundle workers must not set runtime.base_image',
          });
        }
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
