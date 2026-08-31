/**
 * The iii context an agent runs with — fetched, never carried.
 *
 * Both halves of this worker need the same two things: the rules for working
 * against a live engine, and what skills are installed on it. Both live in the
 * `iii-directory` worker, which owns the skills folder on disk and serves it
 * over the bus. So this module asks for them; it holds no copy of either.
 *
 * That is the whole point. A prompt compiled into a worker goes stale the day
 * the engine changes and cannot be edited without a release, and a second copy
 * beside another agent's copy drifts from it silently. One file, one owner, one
 * answer for every agent.
 *
 * When the directory is absent or holds nothing, the answer is empty and the
 * caller says so out loud — a missing context is a fact worth surfacing, not a
 * reason to smuggle a fallback copy back in.
 */

import type { IIIClient } from 'iii-sdk';

/**
 * The system prompt every iii agent reads. Installed in the directory's skills
 * folder (`iii-directory/skills/system-prompts/iii-runtime.md` in this repo,
 * or downloaded with `directory::skills::download*`).
 */
export const RUNTIME_PROMPT_NAME = 'iii-runtime';

const TIMEOUT_MS = 15_000;
/** Long enough that a turn does not pay for the fetch twice, short enough that
 *  an edit to the prompt reaches the next terminal session. */
const CACHE_MS = 60_000;

export type IiiContext = {
  /** The text to give the agent. Empty when the directory served nothing. */
  text: string;
  /** Empty when both parts arrived; otherwise what is missing, and why. */
  detail: string;
};

let cached: { at: number; value: IiiContext } | null = null;

async function systemPrompt(iii: IIIClient): Promise<{ body: string; detail: string }> {
  try {
    const res = await iii.trigger<unknown, { body?: string }>({
      function_id: 'directory::system-prompts::get',
      payload: { name: RUNTIME_PROMPT_NAME },
      timeoutMs: TIMEOUT_MS,
    });
    const body = res?.body?.trim() ?? '';
    return body
      ? { body, detail: '' }
      : {
          body: '',
          detail: `the \`${RUNTIME_PROMPT_NAME}\` system prompt is empty in iii-directory`,
        };
  } catch (err) {
    return {
      body: '',
      detail: `the \`${RUNTIME_PROMPT_NAME}\` system prompt could not be read from iii-directory (${String(err)}); install it with directory::system-prompts::create or directory::skills::download`,
    };
  }
}

async function skillsIndex(iii: IIIClient): Promise<{ body: string; detail: string }> {
  try {
    const res = await iii.trigger<unknown, { body?: string; workers_count?: number }>({
      function_id: 'directory::skills::index',
      payload: {},
      timeoutMs: TIMEOUT_MS,
    });
    const body = res?.body?.trim() ?? '';
    return { body, detail: '' };
  } catch (err) {
    return {
      body: '',
      detail: `the skills index could not be read from iii-directory (${String(err)})`,
    };
  }
}

/**
 * The runtime prompt plus the installed-skills index, as one block of text.
 *
 * Cached for a minute: a headless turn and a terminal boot both ask for it, and
 * neither should pay for two bus calls on every prompt.
 */
export async function fetchIiiContext(iii: IIIClient): Promise<IiiContext> {
  const now = Date.now();
  if (cached && now - cached.at < CACHE_MS) return cached.value;

  const [prompt, skills] = await Promise.all([systemPrompt(iii), skillsIndex(iii)]);
  const parts = [prompt.body, skills.body].filter(Boolean);
  const value: IiiContext = {
    text: parts.join('\n\n'),
    detail: [prompt.detail, skills.detail].filter(Boolean).join('; '),
  };
  cached = { at: now, value };
  if (value.detail) console.warn(`claude-code: iii context incomplete: ${value.detail}`);
  return value;
}

/** Test seam: drop the cache so the next call asks the directory again. */
export function resetIiiContextCache(): void {
  cached = null;
}
