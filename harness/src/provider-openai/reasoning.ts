/**
 * OpenAI reasoning-effort selection. Detects reasoning models (catalog
 * `supports_thinking` flag first, id pattern fallback) and maps the
 * harness `thinking_level` onto a `reasoning_effort` valid for the model
 * family, defaulting to `medium`.
 */

const EFFORT_ORDER = ['none', 'minimal', 'low', 'medium', 'high', 'xhigh'];

export function isReasoningModel(model: string, catalogSupportsThinking?: boolean): boolean {
  if (typeof catalogSupportsThinking === 'boolean') return catalogSupportsThinking;
  return /^(gpt-5|o1|o3|o4)/i.test(model);
}

/** Efforts the model family accepts; empty = don't send the param. */
function supportedEfforts(model: string): string[] {
  const id = model.toLowerCase();
  if (!/^(gpt-5|o1|o3|o4)/.test(id)) return [];
  // The o1 family (o1, o1-mini, o1-preview, o1-pro) rejects
  // reasoning_effort on Chat Completions with a 400 — even though
  // catalogs flag it as a reasoning model. Omit the param entirely.
  if (id.startsWith('o1')) return [];
  // Chat-tuned variants only support the fixed default; omit the param.
  if (id.includes('chat')) return [];
  // gpt-5-pro / gpt-5.x-pro: high only.
  if (id.includes('pro')) return ['high'];
  const minorMatch = id.match(/^gpt-5\.(\d+)/);
  if (minorMatch) {
    const minor = Number(minorMatch[1]);
    // gpt-5.1: none/low/medium/high; gpt-5.2+ adds xhigh.
    return minor >= 2
      ? ['none', 'low', 'medium', 'high', 'xhigh']
      : ['none', 'low', 'medium', 'high'];
  }
  // gpt-5 base family (mini/nano/codex).
  if (id.startsWith('gpt-5')) return ['minimal', 'low', 'medium', 'high'];
  // o-series.
  return ['low', 'medium', 'high'];
}

function normalizeLevel(level: string | undefined): string {
  if (level === undefined) return 'medium';
  if (level === 'off') return 'none';
  if (level === 'max') return 'xhigh';
  return level;
}

/**
 * Effort for a reasoning model: the requested level when the family
 * supports it, else the nearest supported effort below (then above).
 * Returns undefined when the family takes no effort param.
 */
export function reasoningEffortFor(level: string | undefined, model: string): string | undefined {
  const ladder = supportedEfforts(model);
  if (ladder.length === 0) return undefined;
  const want = normalizeLevel(level);
  if (ladder.includes(want)) return want;
  const wantIdx = EFFORT_ORDER.indexOf(want);
  if (wantIdx < 0) return ladder.includes('medium') ? 'medium' : ladder[ladder.length - 1];
  for (let i = wantIdx - 1; i >= 0; i--) {
    const candidate = EFFORT_ORDER[i];
    if (candidate !== undefined && ladder.includes(candidate)) return candidate;
  }
  for (let i = wantIdx + 1; i < EFFORT_ORDER.length; i++) {
    const candidate = EFFORT_ORDER[i];
    if (candidate !== undefined && ladder.includes(candidate)) return candidate;
  }
  return undefined;
}
