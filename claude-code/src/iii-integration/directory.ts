/**
 * The engine's `iii-directory` worker, as the plugin sees it.
 *
 * One lazily-opened iii-sdk connection serves every request. It is opened on
 * the first call, not at server start, so a `claude` started before the engine
 * still comes up cleanly and attaches the moment someone asks for a profile.
 * A failed attempt is not cached: the next call resolves and connects again.
 */

import { type IIIClient, registerWorker } from 'iii-sdk';
import { type Resolved, resolveEngine } from './engine.js';

const CALL_TIMEOUT_MS = 15_000;
const CONNECT_TIMEOUT_MS = 5_000;

export type AgentRow = {
  id: string;
  name?: string;
  description?: string;
  hidden?: boolean;
  extends?: string | null;
  model?: string | null;
  reasoning_effort?: string | null;
  skill_count?: number | null;
  function_count?: number;
};

export type AgentProfile = AgentRow & {
  system_prompt?: string;
  skills?: string[];
  functions?: string[];
  unknown_skills?: string[];
  unknown_functions?: string[];
  inheritance_error?: string;
};

export type SkillRow = { id: string; title?: string; type?: string | null; description?: string };
export type Skill = SkillRow & { body?: string };

export type Caller = <T>(function_id: string, payload: Record<string, unknown>) => Promise<T>;

export class Directory {
  private client: IIIClient | null = null;
  private engine: Resolved | null = null;
  private connecting: Promise<IIIClient> | null = null;

  constructor(
    private readonly resolve: () => Promise<Resolved> = () => resolveEngine(),
    private readonly connect: (url: string) => IIIClient = (url) =>
      registerWorker(url, {
        workerName: 'iii-integration',
        invocationTimeoutMs: CALL_TIMEOUT_MS,
        // Bounded: a standalone CLI must not retry a dead engine forever.
        reconnectionConfig: { maxRetries: 3, maxDelayMs: 5_000 },
        // A plugin is a reader, not a service worth tracing.
        otel: { enabled: false },
      }),
  ) {}

  /** Where the plugin is (or would be) connected, for `iii_status`. */
  status(): { engine: Resolved | null; state: string } {
    return { engine: this.engine, state: this.client?.getConnectionState() ?? 'disconnected' };
  }

  /** Resolve and connect now, without making a call. */
  async connectNow(): Promise<void> {
    await this.ready();
  }

  private async ready(): Promise<IIIClient> {
    const state = this.client?.getConnectionState();
    if (this.client && state !== 'failed') return this.client;
    if (this.client) {
      await this.client.shutdown().catch(() => {});
      this.client = null;
    }
    this.connecting ??= (async () => {
      const engine = await this.resolve();
      this.engine = engine;
      if (!engine.url) throw new Error(engine.source === 'none' ? engine.detail : 'no engine');
      const client = this.connect(engine.url);
      const deadline = Date.now() + CONNECT_TIMEOUT_MS;
      while (client.getConnectionState() !== 'connected') {
        const s = client.getConnectionState();
        if (s === 'failed' || Date.now() > deadline) {
          await client.shutdown().catch(() => {});
          throw new Error(
            `could not connect to the iii engine at ${engine.url} (${engine.source}): ${s}`,
          );
        }
        await new Promise((r) => setTimeout(r, 50));
      }
      this.client = client;
      return client;
    })().finally(() => {
      this.connecting = null;
    });
    return this.connecting;
  }

  readonly call: Caller = async <T>(function_id: string, payload: Record<string, unknown>) => {
    const client = await this.ready();
    return client.call<unknown, T>({ function_id, payload, timeoutMs: CALL_TIMEOUT_MS });
  };

  async listAgents(includeHidden = false): Promise<AgentRow[]> {
    const res = await this.call<{ agents?: AgentRow[] }>('directory::agents::list', {});
    const agents = res?.agents ?? [];
    return includeHidden ? agents : agents.filter((a) => !a.hidden);
  }

  getAgent(id: string): Promise<AgentProfile> {
    return this.call<AgentProfile>('directory::agents::get', { id });
  }

  async listSkills(filter: { search?: string; prefix?: string } = {}): Promise<SkillRow[]> {
    const res = await this.call<{ skills?: SkillRow[] } | SkillRow[]>(
      'directory::skills::list',
      filter,
    );
    return Array.isArray(res) ? res : (res?.skills ?? []);
  }

  getSkill(id: string): Promise<Skill> {
    return this.call<Skill>('directory::skills::get', { id });
  }

  async close(): Promise<void> {
    await this.client?.shutdown().catch(() => {});
    this.client = null;
  }
}

/** A slug Claude Code accepts as an agent or skill name. */
export function slug(id: string): string {
  return id
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

function yamlString(value: string): string {
  return JSON.stringify(value.replace(/\s+/g, ' ').trim());
}

/**
 * An agent profile as a Claude Code subagent file (`agents/<name>.md`).
 *
 * The model is not carried over: iii profiles name models across every
 * provider, and a subagent with a model Claude Code cannot run fails to start.
 * Preloaded skills become an instruction to read them, because their bodies
 * live in the directory and stay there.
 */
export function renderSubagent(profile: AgentProfile): string {
  const name = `iii-${slug(profile.id)}`;
  const description = profile.description || profile.name || profile.id;
  const skills = profile.skills ?? [];
  const preload = skills.length
    ? `\n\n## Preloaded skills\n\nBefore starting, read each of these with the \`iii_get_skill\` tool: ${skills
        .map((s) => `\`${s}\``)
        .join(', ')}.`
    : '';
  return `---
name: ${name}
description: ${yamlString(`${description} (iii agent profile \`${profile.id}\`)`)}
---

${(profile.system_prompt ?? '').trim()}${preload}
`;
}

/** A directory skill as a Claude Code skill file (`skills/<name>/SKILL.md`). */
export function renderSkill(skill: Skill): string {
  const description = skill.description || skill.title || skill.id;
  return `---
name: ${slug(skill.id)}
description: ${yamlString(description)}
---

${(skill.body ?? '').trim()}
`;
}
