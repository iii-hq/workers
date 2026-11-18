/**
 * The live side of a child session's transcript: what the console renders
 * while a Claude Code sub-agent is still working, the way it renders a harness
 * chat.
 *
 * The console builds a chat from session-manager alone — `session::append`
 * adds an entry (`session::on-message-added`), `session::update-message`
 * replaces its content (`session::on-message-updated`, highest revision wins).
 * So the prompt is written the moment the turn starts, each model message is
 * ONE assistant entry that grows as the SDK streams it (`includePartialMessages`)
 * and is finished with the complete content and usage, and each tool result is
 * written as it lands.
 *
 * One model message can reach us as several SDK `assistant` messages sharing
 * an id (one per content block), so entries are keyed by message id, never by
 * SDK message.
 *
 * Best-effort like the rest of the child-session link: writes run in order on
 * one queue, the first failure stops the writer, and nothing here can fail
 * the run.
 */

import { randomUUID } from 'node:crypto';
import type { IIIClient } from 'iii-sdk';
import { makeAssistantMessage, toolFunctionId } from './map.js';
import { toSessionMessage } from './session-link.js';
import type { AssistantMessage, ContentBlock, FunctionResultMessage, Usage } from './types.js';

const TIMEOUT_MS = 15_000;

/** How often a growing message is rewritten. `0` writes every change at once (tests). */
export const streamTiming = { throttleMs: 150 };

/** What runReserved reports as the turn happens — before any feed publish or save. */
export type TurnSink = {
  /** Whether this sink wants the SDK's partial (stream) messages. */
  readonly streaming: boolean;
  streamEvent(event: unknown): void;
  assistant(
    id: string | undefined,
    content: ContentBlock[],
    model: string,
    usage: Usage | null,
  ): void;
  functionResult(message: FunctionResultMessage): void;
  /** The turn's own closing message (the fallback when nothing else was said). */
  terminal(message: AssistantMessage): void;
};

type Entry = {
  message: Record<string, unknown>;
  /** Chosen here, so a retried append is idempotent in session-manager. */
  entryId: string;
  /** An append was sent at least once (its outcome may be unknown). */
  attempted: boolean;
  committed: boolean;
  /** Changed since it was last written. */
  dirty: boolean;
};

type Draft = {
  entry?: Entry;
  model: string;
  usage: Usage | null;
  /** Blocks as the stream builds them, by block index. */
  streamed: Array<ContentBlock | undefined>;
  partialJson: Map<number, string>;
  /** Blocks from the SDK's complete assistant messages — authoritative once streaming ends. */
  final: ContentBlock[];
  streaming: boolean;
  /** The final output count from the stream's message_delta; the SDK's own
   *  usage is a snapshot from the start of the message. */
  outputTokens?: number;
};

type StreamEvent = {
  type?: string;
  index?: number;
  message?: { id?: string; model?: string };
  content_block?: { type?: string; text?: string; thinking?: string; id?: string; name?: string };
  delta?: { type?: string; text?: string; thinking?: string; partial_json?: string };
  usage?: { output_tokens?: number };
};

function visible(block: ContentBlock | undefined): block is ContentBlock {
  if (!block) return false;
  if (block.type === 'text' || block.type === 'thinking') return block.text.length > 0;
  return true;
}

export class ChildTranscript implements TurnSink {
  private readonly entries: Entry[] = [];
  private readonly drafts = new Map<string, Draft>();
  private current: Draft | undefined;
  private chain: Promise<void> = Promise.resolve();
  private failed = false;
  private timer: ReturnType<typeof setTimeout> | undefined;
  private terminalWritten = false;
  private anonymous = 0;

  /**
   * `live` writes as the turn happens. A writer that is not live (ownership
   * could not be read yet) only keeps the transcript, and writes it whole at
   * `finish` if the session turns out to be this worker's child.
   */
  constructor(
    private readonly iii: IIIClient,
    private readonly sessionId: string,
    private live: boolean,
    prompt: string,
  ) {
    this.add({ role: 'user', content: [{ type: 'text', text: prompt }], timestamp: Date.now() });
  }

  get streaming(): boolean {
    return this.live;
  }

  streamEvent(raw: unknown): void {
    const event = (raw ?? {}) as StreamEvent;
    if (event.type === 'message_start') {
      const draft = this.draft(event.message?.id, event.message?.model ?? '');
      draft.streaming = true;
      this.current = draft;
      return;
    }
    const draft = this.current;
    if (!draft) return;
    const index = event.index ?? 0;
    if (event.type === 'content_block_start') {
      const cb = event.content_block ?? {};
      if (cb.type === 'text') draft.streamed[index] = { type: 'text', text: cb.text ?? '' };
      else if (cb.type === 'thinking') {
        draft.streamed[index] = { type: 'thinking', text: cb.thinking ?? '' };
      } else if (cb.type === 'tool_use' && cb.id && cb.name) {
        draft.streamed[index] = {
          type: 'function_call',
          id: cb.id,
          function_id: toolFunctionId(cb.name),
          arguments: {},
        };
      }
    } else if (event.type === 'content_block_delta') {
      const block = draft.streamed[index];
      const delta = event.delta ?? {};
      if (block?.type === 'text' && delta.type === 'text_delta') block.text += delta.text ?? '';
      else if (block?.type === 'thinking' && delta.type === 'thinking_delta') {
        block.text += delta.thinking ?? '';
      } else if (block?.type === 'function_call' && delta.type === 'input_json_delta') {
        draft.partialJson.set(
          index,
          (draft.partialJson.get(index) ?? '') + (delta.partial_json ?? ''),
        );
        return; // arguments render once they parse, at block stop
      } else return;
    } else if (event.type === 'content_block_stop') {
      const block = draft.streamed[index];
      const json = draft.partialJson.get(index);
      if (block?.type === 'function_call' && json) {
        try {
          block.arguments = JSON.parse(json);
        } catch {
          // the complete assistant message carries the real arguments
        }
      }
    } else if (event.type === 'message_delta') {
      if (typeof event.usage?.output_tokens !== 'number') return;
      draft.outputTokens = event.usage.output_tokens;
    } else if (event.type === 'message_stop') {
      draft.streaming = false;
      this.current = undefined;
    } else return;
    this.refresh(draft);
  }

  assistant(
    id: string | undefined,
    content: ContentBlock[],
    model: string,
    usage: Usage | null,
  ): void {
    const draft = this.draft(id, model);
    draft.final.push(...content);
    if (usage) draft.usage = usage;
    if (model) draft.model = model;
    this.refresh(draft);
  }

  functionResult(message: FunctionResultMessage): void {
    this.add(toSessionMessage(message as unknown as Record<string, unknown>));
  }

  terminal(message: AssistantMessage): void {
    this.terminalWritten = true;
    this.add(toSessionMessage(message as unknown as Record<string, unknown>));
  }

  /**
   * Close the turn with how it ended, unless the turn already said so itself.
   * A stop reason cannot be rewritten on an entry already written, so a turn
   * that hit a cap or failed ends with its own short assistant message.
   */
  note(stop: string, text: string): void {
    if (this.terminalWritten) return;
    this.terminal(makeAssistantMessage([{ type: 'text', text }], '', null, stop));
  }

  /**
   * Write whatever is still pending and wait for it. A writer that was not
   * live asks `confirm` (ownership, again) and writes the whole turn only on
   * yes — `confirm` is where the caller moves the status to `working` first.
   * `write`: the status of this session should move. `complete`: every entry
   * committed, so the transcript is whole.
   */
  async finish(confirm: () => Promise<boolean>): Promise<{ write: boolean; complete: boolean }> {
    if (this.timer) clearTimeout(this.timer);
    this.timer = undefined;
    for (const draft of this.drafts.values()) {
      draft.streaming = false;
      this.refresh(draft, false);
    }
    if (!this.live) {
      if (!(await confirm())) return { write: false, complete: false };
      this.live = true;
      for (const entry of this.entries) this.enqueue(() => this.append(entry));
    } else {
      this.flush();
    }
    await this.chain;
    return { write: true, complete: !this.failed };
  }

  private draft(id: string | undefined, model: string): Draft {
    const key = id ?? `anonymous-${++this.anonymous}`;
    let draft = this.drafts.get(key);
    if (!draft) {
      draft = {
        model,
        usage: null,
        streamed: [],
        partialJson: new Map(),
        final: [],
        streaming: false,
      };
      this.drafts.set(key, draft);
    }
    return draft;
  }

  /** What a draft shows: the live stream while it runs, the SDK's complete blocks after. */
  private display(draft: Draft): ContentBlock[] {
    const streamed = draft.streamed.filter(visible).map((b) => ({ ...b }));
    if (draft.streaming) return streamed.length ? streamed : [...draft.final];
    return draft.final.length ? [...draft.final] : streamed;
  }

  private refresh(draft: Draft, schedule = true): void {
    const content = this.display(draft);
    if (content.length === 0) return;
    const usage: Usage | null =
      draft.outputTokens === undefined
        ? draft.usage
        : { input_tokens: 0, ...draft.usage, output_tokens: draft.outputTokens };
    const message = toSessionMessage(
      makeAssistantMessage(content, draft.model, usage) as unknown as Record<string, unknown>,
    );
    if (!draft.entry) {
      draft.entry = this.add(message);
      return;
    }
    draft.entry.message = { ...draft.entry.message, content, usage: message.usage };
    draft.entry.dirty = true;
    if (schedule) this.schedule();
  }

  private add(message: Record<string, unknown>): Entry {
    const entry: Entry = {
      message,
      entryId: `cc-${randomUUID()}`,
      attempted: false,
      committed: false,
      dirty: false,
    };
    this.entries.push(entry);
    if (this.live) this.enqueue(() => this.append(entry));
    return entry;
  }

  private schedule(): void {
    if (!this.live) return;
    if (streamTiming.throttleMs <= 0) {
      this.flush();
      return;
    }
    if (this.timer) return;
    this.timer = setTimeout(() => {
      this.timer = undefined;
      this.flush();
    }, streamTiming.throttleMs);
  }

  private flush(): void {
    for (const entry of this.entries) {
      if (entry.dirty) this.enqueue(() => this.update(entry));
    }
  }

  /** One ordered queue; each write gets one retry, then the writer stops. */
  private enqueue(op: () => Promise<void>): void {
    this.chain = this.chain.then(async () => {
      if (this.failed) return;
      try {
        await op();
      } catch {
        try {
          await op();
        } catch (err) {
          this.failed = true;
          console.warn(
            `claude-code: child transcript for ${this.sessionId} stopped: ${String(err)}`,
          );
        }
      }
    });
  }

  private async append(entry: Entry): Promise<void> {
    // The append carries the entry as it is NOW, so a change made while it
    // waited in the queue needs no separate update. The caller-chosen entry id
    // makes a retry of an append that did commit a no-op, never a duplicate.
    const retry = entry.attempted;
    entry.attempted = true;
    entry.dirty = false;
    try {
      await this.iii.call({
        function_id: 'session::append',
        payload: { session_id: this.sessionId, entry_id: entry.entryId, message: entry.message },
        timeoutMs: TIMEOUT_MS,
      });
    } catch (err) {
      // What the store holds is unknown: whatever it is, write it again.
      entry.dirty = true;
      throw err;
    }
    entry.committed = true;
    if (retry) {
      // A first attempt that DID commit stored an older snapshot, and the
      // idempotent retry returns that entry unchanged: write the content as it
      // is now (a full replacement, so harmless when the retry stored it).
      entry.dirty = true;
      await this.update(entry);
    }
  }

  private async update(entry: Entry): Promise<void> {
    if (!entry.dirty || !entry.committed) return;
    entry.dirty = false;
    const usage = entry.message.usage;
    try {
      await this.iii.call({
        function_id: 'session::update-message',
        payload: {
          session_id: this.sessionId,
          entry_id: entry.entryId,
          content: entry.message.content,
          ...(usage ? { usage } : {}),
        },
        timeoutMs: TIMEOUT_MS,
      });
    } catch (err) {
      entry.dirty = true;
      throw err;
    }
  }
}
