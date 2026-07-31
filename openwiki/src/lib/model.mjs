// Resolve a model id against the live router catalog. The harness and its
// output contract need a real (model, provider) pair, and structured-output
// support decides whether the contract rides provider-native JSON or the
// harness's submit_result fallback. Never hardcode a model id — validate it.

const cache = new Map(); // preferred -> resolved (per worker run)

export function pickModel(models, preferred) {
  const list = Array.isArray(models) ? models : [];
  const byId = (id) => list.find((m) => m && m.id === id);
  let m = preferred ? byId(preferred) : null;
  if (!m && list.length) {
    m =
      list.find((x) => x.supports_structured_output && x.supports_tools) ||
      list.find((x) => x.supports_tools) ||
      list[0];
  }
  if (!m) {
    return { model: preferred || null, provider: undefined, supports_structured_output: false, resolved: false };
  }
  return {
    model: m.id,
    provider: m.provider,
    supports_structured_output: !!m.supports_structured_output,
    resolved: true,
  };
}

export async function resolveModel(worker, preferred) {
  const key = preferred || '';
  if (cache.has(key)) return cache.get(key);

  let models = [];
  try {
    const res = await worker.trigger({ function_id: 'router::models::list', payload: {} });
    models = res?.models || (Array.isArray(res) ? res : []);
  } catch {
    // router unavailable — fall back to the preferred id unresolved; harness
    // will error and the caller drops to the router/heuristic page path.
  }
  const out = pickModel(models, preferred);
  cache.set(key, out);
  return out;
}

export function clearModelCache() {
  cache.clear();
}

// The router's live model catalog, trimmed to what the UI's picker needs.
// Returns [] when llm-router is absent (no provider configured), which the UI
// treats as "set up a provider in the console".
export async function listModels(worker) {
  let models = [];
  try {
    const res = await worker.trigger({ function_id: 'router::models::list', payload: {} });
    models = res?.models || (Array.isArray(res) ? res : []);
  } catch {
    return [];
  }
  return models
    .filter((m) => m?.id)
    .map((m) => ({
      id: m.id,
      provider: m.provider || 'unknown',
      supports_structured_output: !!m.supports_structured_output,
    }));
}
