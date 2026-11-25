/**
 * A stored configuration written by an older release must keep validating
 * after a release adds a setting. The configuration worker validates the
 * stored value against the schema this worker publishes, and compose refuses
 * to start a worker whose configuration does not validate, so the worker
 * never gets to fill in the new setting's default itself. A setting with a
 * default is therefore optional in the published schema; the worker's own
 * parse supplies the default.
 */
import { describe, expect, it } from 'vitest';
import { RuntimeConfigSchema, runtimeJsonSchema } from '../src/config.js';

type Node = {
  type?: string | string[];
  properties?: Record<string, Node>;
  required?: string[];
  default?: unknown;
  anyOf?: Node[];
  oneOf?: Node[];
};

/** Every object node in the schema, with its path. */
function objects(node: Node, path = '$'): { path: string; node: Node }[] {
  const out: { path: string; node: Node }[] = [];
  if (node.properties) out.push({ path, node });
  for (const [key, child] of Object.entries(node.properties ?? {})) {
    out.push(...objects(child, `${path}.${key}`));
  }
  for (const child of [...(node.anyOf ?? []), ...(node.oneOf ?? [])])
    out.push(...objects(child, path));
  return out;
}

/** The required keys a value is missing, at every object level the schema describes. */
function missingRequired(node: Node, value: unknown, path = '$'): string[] {
  if (!node.properties || value === null || typeof value !== 'object' || Array.isArray(value))
    return [];
  const v = value as Record<string, unknown>;
  const missing = (node.required ?? []).filter((k) => !(k in v)).map((k) => `${path}.${k}`);
  for (const [key, child] of Object.entries(node.properties)) {
    if (key in v) missing.push(...missingRequired(child, v[key], `${path}.${key}`));
  }
  return missing;
}

/** The stored default-claude-code of a project created before external_sessions existed (#1898). */
const BEFORE_EXTERNAL_SESSIONS = {
  approval_gate: false,
  claude_executable: '/Users/u/.local/bin/claude',
  defaults: {
    allowed_tools: [],
    append_system_prompt: '',
    cwd: '',
    disallowed_tools: [],
    max_turns: 50,
    model: '',
    permission_mode: 'acceptEdits',
  },
  iii_context: true,
  terminal: {
    args: [],
    auto_install: true,
    executable: '',
    setup_workspace: true,
    workspace_dir: '',
  },
};

describe('a configuration stored by an older release', () => {
  it('still validates after external_sessions was added', () => {
    expect(missingRequired(runtimeJsonSchema() as Node, BEFORE_EXTERNAL_SESSIONS)).toEqual([]);
  });

  it('gets the new setting from its default when the worker reads it', () => {
    expect(RuntimeConfigSchema.parse(BEFORE_EXTERNAL_SESSIONS).external_sessions).toBe(false);
  });

  it('still validates with only some settings stored, at every level', () => {
    expect(missingRequired(runtimeJsonSchema() as Node, { defaults: {}, terminal: {} })).toEqual(
      [],
    );
    expect(missingRequired(runtimeJsonSchema() as Node, {})).toEqual([]);
  });
});

describe('the published schema', () => {
  it('lists external_sessions as optional, with its default', () => {
    const schema = runtimeJsonSchema() as Node;
    expect(schema.required ?? []).not.toContain('external_sessions');
    expect(schema.properties?.external_sessions?.default).toBe(false);
  });

  it('requires no setting that has a default, anywhere: a future setting cannot break old configurations', () => {
    const offenders = objects(runtimeJsonSchema() as Node).flatMap(({ path, node }) =>
      (node.required ?? [])
        .filter((key) => node.properties?.[key]?.default !== undefined)
        .map((key) => `${path}.${key}`),
    );
    expect(offenders).toEqual([]);
  });

  it('still describes every setting, so the console form and validation see them all', () => {
    const props = Object.keys((runtimeJsonSchema() as Node).properties ?? {});
    for (const key of [
      'approval_gate',
      'claude_executable',
      'defaults',
      'iii_context',
      'external_sessions',
      'terminal',
    ]) {
      expect(props).toContain(key);
    }
  });
});
