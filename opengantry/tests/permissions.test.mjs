/**
 * The registration hooks are bus-internal: agents must not call them. The
 * harness loads the repository-root iii-permissions.yaml, so every deny in
 * this worker's own file must hold there too, ahead of any rule that would
 * allow the id first (rules are first-match-wins).
 */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { test } from 'node:test';

import { FUNCTION_FORMATS } from '../src/formats.js';

const WORKER_FILE = new URL('../iii-permissions.yaml', import.meta.url);
const ROOT_FILE = new URL('../../iii-permissions.yaml', import.meta.url);

/**
 * The top-level entries of `rules:` in order: a string rule as written
 * (`'!id'` denies, anything else allows), an object rule as its `function`.
 */
function rulesOf(file) {
  const rules = [];
  let inRules = false;
  for (const line of fs.readFileSync(file, 'utf8').split('\n')) {
    if (/^rules:\s*$/.test(line)) {
      inRules = true;
      continue;
    }
    if (!inRules) continue;
    if (/^\S/.test(line)) break;
    const item = /^ {2}- (.*?)\s*$/.exec(line);
    if (!item) continue;
    const object = /^function:\s*(.*)$/.exec(item[1]);
    const text = unquote(object ? object[1] : item[1]);
    rules.push(object ? { allow: text, conditional: true } : ruleOf(text));
  }
  return rules;
}

function unquote(text) {
  return text.replace(/^(['"])(.*)\1$/, '$2');
}

function ruleOf(text) {
  return text.startsWith('!') ? { deny: text.slice(1) } : { allow: text };
}

function matches(pattern, id) {
  const re = new RegExp(`^${pattern.split('*').map(escapeRegExp).join('.*')}$`);
  return re.test(id);
}

function escapeRegExp(text) {
  return text.replace(/[.+?^${}()|[\]\\]/g, '\\$&');
}

/** First-match-wins: the first rule whose pattern matches the id decides. */
function decide(rules, id) {
  for (const rule of rules) {
    if (rule.deny !== undefined && matches(rule.deny, id)) return 'deny';
    if (rule.allow !== undefined && matches(rule.allow, id)) return 'allow';
  }
  return 'needs_approval';
}

const HOOKS = Object.keys(FUNCTION_FORMATS).filter((id) => id.includes('-registration'));

test('this worker denies agents both registration hooks', () => {
  assert.deepEqual([...HOOKS].sort(), [
    'gantry::on-function-registration',
    'gantry::on-observer-registration',
  ]);
  const rules = rulesOf(WORKER_FILE);
  for (const id of HOOKS) assert.equal(decide(rules, id), 'deny', id);
});

test('the repository-root policy the harness loads denies them too', () => {
  const rules = rulesOf(ROOT_FILE);
  assert.ok(rules.length > 0, 'the root file has rules');
  for (const rule of rulesOf(WORKER_FILE)) {
    assert.ok(rule.deny !== undefined, `${JSON.stringify(rule)} is a deny`);
    assert.equal(decide(rules, rule.deny), 'deny', rule.deny);
  }
});
