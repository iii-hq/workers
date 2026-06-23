/**
 * Build a Pi AgentSession for one turn. Pi runs the agent loop in-process via
 * `@earendil-works/pi-coding-agent`, so there is no CLI subprocess: the worker
 * owns auth, the model registry, and the session manager directly.
 *
 * Resume is by session file: a prior run stores its `sessionFile`, and the next
 * run for the same iii session_id opens it so the Pi conversation continues.
 */

import {
  type AgentSession,
  AuthStorage,
  createAgentSession,
  ModelRegistry,
  SessionManager,
} from '@earendil-works/pi-coding-agent';

export type ThinkingLevel = 'off' | 'minimal' | 'low' | 'medium' | 'high' | 'xhigh';

export type BuildSessionOptions = {
  cwd?: string;
  model?: string;
  thinkingLevel: ThinkingLevel;
  tools: string[];
  agentDir?: string;
  resumeFile?: string | null;
};

/** Resolve a "provider/modelId" string against the registry; undefined = Pi default. */
function resolveModel(registry: ModelRegistry, model: string | undefined) {
  if (!model) return undefined;
  const sep = model.indexOf('/');
  if (sep <= 0) return undefined;
  const provider = model.slice(0, sep);
  const modelId = model.slice(sep + 1);
  const resolved = registry.find(provider, modelId);
  if (!resolved) console.warn(`model "${model}" not found in registry; using Pi default`);
  return resolved;
}

export async function buildSession(opts: BuildSessionOptions): Promise<AgentSession> {
  const agentDir = opts.agentDir || undefined;
  const cwd = opts.cwd || process.cwd();
  const authStorage = AuthStorage.create(agentDir);
  const modelRegistry = ModelRegistry.create(authStorage);
  const sessionManager = opts.resumeFile
    ? SessionManager.open(opts.resumeFile)
    : SessionManager.create(cwd);

  const { session } = await createAgentSession({
    cwd,
    agentDir,
    authStorage,
    modelRegistry,
    sessionManager,
    thinkingLevel: opts.thinkingLevel,
    ...(opts.tools.length ? { tools: opts.tools } : {}),
    ...(() => {
      const model = resolveModel(modelRegistry, opts.model);
      return model ? { model } : {};
    })(),
  });
  return session;
}
