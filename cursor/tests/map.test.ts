import {
  mapModels,
  mapStructTokenUsage,
  mapTokenUsage,
  mapUsageResponse,
  RunAccumulator,
} from '../src/map.js';

describe('RunAccumulator', () => {
  it('streams deltas and emits one terminal message without duplicating its body', async () => {
    const events: unknown[] = [];
    const accumulator = new RunAccumulator('session-one', 'composer-2', async (_group, event) => {
      events.push(event);
    });

    await accumulator.ingest({
      interactionUpdate: { type: 'thinking-delta', update: { delta: 'wh' } },
    });
    await accumulator.ingest({
      interactionUpdate: { type: 'text-delta', update: { delta: 'Hel' } },
    });
    await accumulator.ingest({
      sdkMessage: { type: 'thinking', message: { text: 'why' } },
    });
    await accumulator.ingest({
      result: {
        runId: 'run-one',
        status: 'RUN_LIFECYCLE_STATUS_FINISHED',
        result: {
          runId: 'run-one',
          status: 'RUN_LIFECYCLE_STATUS_FINISHED',
          result: 'Hello',
        },
      },
    });

    const terminal = await accumulator.finalize();
    const updates = events.filter(
      (event) => (event as { type?: string }).type === 'message_update',
    ) as Array<{ llm_event: { type: string; delta: string } }>;
    const completes = events.filter(
      (event) => (event as { type?: string }).type === 'message_complete',
    ) as Array<{ body_streamed?: boolean }>;

    expect(
      updates
        .filter((event) => event.llm_event.type === 'text_delta')
        .map((event) => event.llm_event.delta)
        .join(''),
    ).toBe('Hello');
    expect(
      updates
        .filter((event) => event.llm_event.type === 'thinking_delta')
        .map((event) => event.llm_event.delta)
        .join(''),
    ).toBe('why');
    expect(completes).toHaveLength(1);
    expect(completes[0]?.body_streamed).toBe(true);
    expect(terminal.message.stop_reason).toBe('end');
    expect(events.map((event) => (event as { type: string }).type).slice(-3)).toEqual([
      'message_complete',
      'turn_end',
      'agent_end',
    ]);
  });

  it('emits an unstreamed complete body for batch output', async () => {
    const events: unknown[] = [];
    const accumulator = new RunAccumulator('session', 'model', async (_group, event) => {
      events.push(event);
    });
    await accumulator.ingest({
      result: {
        runId: 'run',
        status: 'RUN_LIFECYCLE_STATUS_FINISHED',
        result: {
          runId: 'run',
          status: 'RUN_LIFECYCLE_STATUS_FINISHED',
          result: 'batch reply',
        },
      },
    });
    await accumulator.finalize();

    expect(events.some((event) => (event as { type?: string }).type === 'message_update')).toBe(
      false,
    );
    expect(
      events.find((event) => (event as { type?: string }).type === 'message_complete'),
    ).not.toHaveProperty('body_streamed');
  });

  it('keeps a corrected terminal body when streamed deltas do not match it', async () => {
    const events: unknown[] = [];
    const accumulator = new RunAccumulator('session', 'model', async (_group, event) => {
      events.push(event);
    });
    await accumulator.ingest({
      interactionUpdate: { type: 'text-delta', update: { delta: 'draft' } },
    });
    await accumulator.ingest({
      result: {
        runId: 'run',
        status: 'RUN_LIFECYCLE_STATUS_FINISHED',
        result: {
          runId: 'run',
          status: 'RUN_LIFECYCLE_STATUS_FINISHED',
          result: 'final',
        },
      },
    });
    await accumulator.finalize();

    expect(
      events.find((event) => (event as { type?: string }).type === 'message_complete'),
    ).not.toHaveProperty('body_streamed');
  });

  it('surfaces a terminal error code when no status message is available', async () => {
    const accumulator = new RunAccumulator('session', 'model', async () => undefined);
    await accumulator.ingest({
      result: {
        runId: 'run',
        status: 'RUN_LIFECYCLE_STATUS_ERROR',
        errorCode: 'UPSTREAM_ERROR',
        result: { runId: 'run', status: 'RUN_LIFECYCLE_STATUS_ERROR' },
      },
    });

    const terminal = await accumulator.finalize();

    expect(terminal.message.stop_reason).toBe('error');
    expect(terminal.message.error_message).toBe('UPSTREAM_ERROR');
  });

  it('uses the terminal model reported by Cursor', async () => {
    const accumulator = new RunAccumulator('session', '', async () => undefined);
    await accumulator.ingest({
      result: {
        runId: 'run',
        status: 'RUN_LIFECYCLE_STATUS_FINISHED',
        result: {
          runId: 'run',
          status: 'RUN_LIFECYCLE_STATUS_FINISHED',
          result: 'done',
          model: { id: 'cursor-selected-model' },
        },
      },
    });

    const terminal = await accumulator.finalize();

    expect(accumulator.resolvedModel).toBe('cursor-selected-model');
    expect(terminal.message.model).toBe('cursor-selected-model');
  });

  it('normalizes tool progress and terminal cancellation', async () => {
    const events: unknown[] = [];
    const accumulator = new RunAccumulator('session', 'model', async (_group, event) => {
      events.push(event);
    });
    await accumulator.ingest({
      interactionUpdate: {
        type: 'tool-call-started',
        update: { callId: 'call-one', toolCall: { type: 'read', args: { path: 'a' } } },
      },
    });
    await accumulator.ingest({
      interactionUpdate: {
        type: 'tool-call-completed',
        update: {
          callId: 'call-one',
          toolCall: { type: 'read', result: { status: 'success', value: 'ok' } },
        },
      },
    });
    await accumulator.ingest({
      result: {
        runId: 'run',
        status: 'RUN_LIFECYCLE_STATUS_CANCELLED',
        result: { runId: 'run', status: 'RUN_LIFECYCLE_STATUS_CANCELLED', result: '' },
      },
    });
    const terminal = await accumulator.finalize();

    expect(events).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          type: 'function_execution_start',
          function_call_id: 'call-one',
          function_id: 'cursor::tool::read',
        }),
        expect.objectContaining({
          type: 'function_execution_end',
          function_call_id: 'call-one',
          is_error: false,
        }),
      ]),
    );
    expect(terminal.message.stop_reason).toBe('aborted');
  });
});

describe('wire mappings', () => {
  it('maps token strings without inventing missing reasoning usage', () => {
    expect(
      mapTokenUsage({
        inputTokens: '2',
        outputTokens: '3',
        cacheReadTokens: '1',
        cacheWriteTokens: '0',
        totalTokens: '6',
      }),
    ).toEqual({
      input_tokens: 2,
      output_tokens: 3,
      cache_read_tokens: 1,
      cache_write_tokens: 0,
      total_tokens: 6,
    });
    expect(mapStructTokenUsage(undefined)).toBeNull();
    expect(() =>
      mapTokenUsage({
        inputTokens: '9007199254740992',
        outputTokens: '0',
        cacheReadTokens: '0',
        cacheWriteTokens: '0',
        totalTokens: '9007199254740992',
      }),
    ).toThrow('safe JavaScript integer');
  });

  it('preserves absent usage and cost', () => {
    expect(mapUsageResponse({})).toEqual({ usage: null, cost: null, runs: [] });
    expect(
      mapUsageResponse({
        usage: {
          runs: [
            { runId: 'pending' },
            {
              runId: 'reported',
              usage: {
                inputTokens: '1',
                outputTokens: '2',
                cacheReadTokens: '0',
                cacheWriteTokens: '0',
                totalTokens: '3',
              },
            },
          ],
        },
      }).runs,
    ).toEqual([
      {
        run_id: 'reported',
        usage: {
          input_tokens: 1,
          output_tokens: 2,
          cache_read_tokens: 0,
          cache_write_tokens: 0,
          total_tokens: 3,
        },
        cost: null,
      },
    ]);
  });

  it('maps only model fields returned by Cursor', () => {
    expect(
      mapModels({
        items: [
          {
            id: 'dynamic-model',
            displayName: 'Dynamic',
            description: 'reported',
            ...({ unknownPricing: 10 } as Record<string, unknown>),
          },
        ],
      }),
    ).toEqual([
      {
        id: 'dynamic-model',
        display_name: 'Dynamic',
        description: 'reported',
        parameters: [],
        variants: [],
      },
    ]);
  });
});
