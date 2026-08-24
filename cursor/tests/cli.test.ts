import { EventEmitter } from 'node:events';
import { createInterface } from 'node:readline';
import { PassThrough } from 'node:stream';
import {
  AcpJsonRpcClient,
  type CommandRunner,
  CursorCliError,
  discoverCursorAgentBinary,
  parseCursorAuthStatus,
  ProductionCursorCliFactory,
} from '../src/cli.js';

describe('Cursor CLI discovery and account output', () => {
  it('rejects a Grok agent collision and never falls back to bare agent', async () => {
    const calls: string[] = [];
    const run: CommandRunner = async (command, args) => {
      calls.push(`${command} ${args.join(' ')}`);
      if (command === '/tools/agent') return { stdout: 'grok 0.1.210 (build)', stderr: '' };
      throw new Error('not found');
    };

    await expect(
      discoverCursorAgentBinary('agent', {
        run,
        cwd: '/worker',
        home: '/home/test',
        env: { PATH: '/tools' },
      }),
    ).rejects.toThrow('not the official Cursor CLI');
    calls.length = 0;
    await expect(
      discoverCursorAgentBinary('', {
        run,
        cwd: '/worker',
        home: '/home/test',
        env: { PATH: '/tools' },
      }),
    ).rejects.toThrow('Cursor Agent CLI was not found');
    expect(calls.some((call) => call.startsWith('agent '))).toBe(false);
  });

  it('uses the validated absolute agent fallback when cursor-agent is absent', async () => {
    const calls: string[] = [];
    const run: CommandRunner = async (command, args) => {
      calls.push(`${command} ${args.join(' ')}`);
      if (command === '/home/test/.local/bin/agent' && args[0] === '--version') {
        return { stdout: '2026.08.11-e8db854\n', stderr: '' };
      }
      if (command === '/home/test/.local/bin/agent' && args[0] === 'acp') {
        return { stdout: 'Start the Cursor Agent as an ACP server', stderr: '' };
      }
      throw new Error('not found');
    };

    await expect(
      discoverCursorAgentBinary('', { run, home: '/home/test', env: {} }),
    ).resolves.toEqual({
      binary: '/home/test/.local/bin/agent',
      version: '2026.08.11-e8db854',
    });
    expect(calls[0]).toBe('/home/test/.local/bin/cursor-agent --version');
  });

  it('ignores relative and empty PATH entries during automatic discovery', async () => {
    const commands: string[] = [];
    const run: CommandRunner = async (command) => {
      commands.push(command);
      throw new Error('not found');
    };

    await expect(
      discoverCursorAgentBinary('', {
        run,
        cwd: '/untrusted/repository',
        home: '/home/test',
        env: { PATH: './bin::/trusted/bin:relative-tools' },
      }),
    ).rejects.toThrow('Cursor Agent CLI was not found');

    expect(commands).toContain('/trusted/bin/cursor-agent');
    expect(commands.some((command) => command.startsWith('/untrusted/repository/'))).toBe(false);
  });

  it('checks every PATH candidate for a configured bare binary name', async () => {
    const run: CommandRunner = async (command, args) => {
      if (command === '/good/cursor-agent' && args[0] === '--version') {
        return { stdout: '2026.08.11-e8db854\n', stderr: '' };
      }
      if (command === '/good/cursor-agent' && args[0] === 'acp') {
        return { stdout: 'Start the Cursor Agent as an ACP server', stderr: '' };
      }
      if (command === '/bad/cursor-agent') {
        return { stdout: 'not cursor', stderr: '' };
      }
      throw new Error('not found');
    };

    await expect(
      discoverCursorAgentBinary('cursor-agent', {
        run,
        cwd: '/worker',
        home: '/home/test',
        env: { PATH: '/bad:/good' },
      }),
    ).resolves.toEqual({
      binary: '/good/cursor-agent',
      version: '2026.08.11-e8db854',
    });
  });

  it('pins a configured relative binary before spawning from an untrusted workspace', async () => {
    const process = new FakeAcpProcess();
    serve(process, (message) => {
      if (message.method === 'initialize') {
        process.send({
          jsonrpc: '2.0',
          id: message.id,
          result: { protocolVersion: 1, authMethods: [{ id: 'cursor_login' }] },
        });
      }
    });
    const spawned: Array<{ command: string; cwd: string; path: string | undefined }> = [];
    const factory = new ProductionCursorCliFactory({
      cwd: '/worker',
      home: '/home/test',
      env: { PATH: './bin::/trusted/bin:relative-tools' },
      canonicalize: async (candidate) => {
        expect(candidate).toBe('/worker/cursor-agent');
        return '/opt/cursor/current/cursor-agent';
      },
      run: async (command, args, options) => {
        expect(command).toBe('/opt/cursor/current/cursor-agent');
        expect(options.cwd).toBe('/worker');
        return args[0] === '--version'
          ? { stdout: '2026.08.11-e8db854\n', stderr: '' }
          : { stdout: 'Start the Cursor Agent as an ACP server', stderr: '' };
      },
      spawn: (command, _args, options) => {
        spawned.push({ command, cwd: options.cwd, path: options.env.PATH });
        return process.asCliProcess();
      },
    });

    const client = await factory.create({
      binary: './cursor-agent',
      workspace: '/untrusted/repository',
      startupTimeoutMs: 1_000,
      shutdownTimeoutMs: 100,
      rpcTimeoutMs: 1_000,
      maxFrameBytes: 1_024,
    });

    expect(spawned).toEqual([
      {
        command: '/opt/cursor/current/cursor-agent',
        cwd: '/untrusted/repository',
        path: '/trusted/bin',
      },
    ]);
    await client.close();
  });

  it('derives the public model catalog from a fresh ACP session', async () => {
    const process = new FakeAcpProcess();
    const methods: string[] = [];
    serve(process, (message) => {
      if (typeof message.method === 'string') methods.push(message.method);
      if (message.method === 'initialize') {
        process.send({
          jsonrpc: '2.0',
          id: message.id,
          result: { protocolVersion: 1, authMethods: [{ id: 'cursor_login' }] },
        });
      } else if (message.method === 'session/new') {
        process.send({
          jsonrpc: '2.0',
          id: message.id,
          result: {
            sessionId: 'catalog-probe',
            models: {
              currentModelId: 'default',
              availableModels: [
                { modelId: 'default', name: 'Auto' },
                { modelId: 'composer-2.5', name: 'Composer 2.5' },
              ],
            },
          },
        });
      }
    });
    const commandArgs: string[][] = [];
    const factory = new ProductionCursorCliFactory({
      cwd: '/worker',
      home: '/home/test',
      env: { PATH: '/tools' },
      canonicalize: async () => '/tools/cursor-agent',
      run: async (_command, args) => {
        commandArgs.push(args);
        return args[0] === '--version'
          ? { stdout: '2026.08.11-e8db854\n', stderr: '' }
          : { stdout: 'Start the Cursor Agent as an ACP server', stderr: '' };
      },
      spawn: () => process.asCliProcess(),
    });

    await expect(
      factory.listModels({
        binary: '/tools/cursor-agent',
        workspace: '/repo',
        startupTimeoutMs: 1_000,
        shutdownTimeoutMs: 100,
        rpcTimeoutMs: 1_000,
        maxFrameBytes: 1_024,
      }),
    ).resolves.toEqual([
      {
        id: 'auto',
        display_name: 'Auto',
        description: '',
        parameters: [],
        variants: [],
      },
      {
        id: 'composer-2.5',
        display_name: 'Composer 2.5',
        description: '',
        parameters: [],
        variants: [],
      },
    ]);
    expect(methods).toEqual(['initialize', 'session/new']);
    expect(commandArgs).not.toContainEqual(['--list-models']);
    expect(process.signals).toEqual(['SIGTERM']);
  });

  it('returns only redacted authentication state', () => {
    const status = parseCursorAuthStatus(
      JSON.stringify({
        hasAccessToken: true,
        hasRefreshToken: false,
        isAuthenticated: false,
        status: 'partial',
        userInfo: { email: 'private@example.com', accessToken: 'secret-token' },
      }),
      '2026.08.11-e8db854',
    );
    expect(status).toEqual({
      authenticated: false,
      status: 'partial',
      version: '2026.08.11-e8db854',
      login_command: 'cursor-agent login',
    });
    expect(JSON.stringify(status)).not.toContain('private@example.com');
    expect(JSON.stringify(status)).not.toContain('secret-token');
    expect(parseCursorAuthStatus('{}', '2026.08.11-e8db854').status).toBe('unauthenticated');
  });
});

describe('Cursor ACP JSON-RPC transport', () => {
  it('filters updates to the active session and cancels permission requests', async () => {
    const process = new FakeAcpProcess();
    const outbound: Array<Record<string, unknown>> = [];
    let promptId: number | string | null = null;
    serve(process, (message) => {
      outbound.push(message);
      if (message.method === 'initialize') {
        process.send({
          jsonrpc: '2.0',
          id: message.id,
          result: { protocolVersion: 1, authMethods: [{ id: 'cursor_login' }] },
        });
      } else if (message.method === 'session/prompt') {
        promptId = requestId(message);
        process.send(update('other-session', 'agent_message_chunk', 'ignored'));
        process.send(update('session-one', 'agent_thought_chunk', 'thinking'));
        process.send({
          jsonrpc: '2.0',
          id: 'permission-one',
          method: 'session/request_permission',
          params: { sessionId: 'session-one', options: [] },
        });
        process.send(update('session-one', 'agent_message_chunk', 'answer'));
      } else if (message.id === 'permission-one') {
        process.send({
          jsonrpc: '2.0',
          id: promptId,
          result: { stopReason: 'end_turn' },
        });
      }
    });
    const client = new AcpJsonRpcClient(process.asCliProcess(), 1_000, 1_000);
    await client.initialize();
    const received: unknown[] = [];

    await expect(
      client.prompt('session-one', 'hello', async (event) => {
        received.push(event);
      }),
    ).resolves.toBe('end_turn');

    expect(received).toHaveLength(2);
    expect(JSON.stringify(received)).not.toContain('ignored');
    expect(outbound.find((message) => message.id === 'permission-one')).toEqual({
      jsonrpc: '2.0',
      id: 'permission-one',
      result: { outcome: { outcome: 'cancelled' } },
    });
    await client.close();
  });

  it('sends cancellation and rejects overlapping prompts', async () => {
    const process = new FakeAcpProcess();
    let promptId: number | string | null = null;
    serve(process, (message) => {
      if (message.method === 'initialize') {
        process.send({
          jsonrpc: '2.0',
          id: message.id,
          result: { protocolVersion: 1, authMethods: [{ id: 'cursor_login' }] },
        });
      } else if (message.method === 'session/prompt') {
        promptId = requestId(message);
      } else if (message.method === 'session/cancel') {
        process.send({
          jsonrpc: '2.0',
          id: promptId,
          result: { stopReason: 'cancelled' },
        });
      }
    });
    const client = new AcpJsonRpcClient(process.asCliProcess(), 1_000, 1_000);
    await client.initialize();
    const first = client.prompt('session-one', 'wait', async () => undefined);
    await vi.waitFor(() => expect(promptId).not.toBeNull());

    await expect(client.prompt('session-two', 'overlap', async () => undefined)).rejects.toThrow(
      'already has an active prompt',
    );
    await expect(client.cancel('session-one')).resolves.toBeUndefined();
    await expect(first).resolves.toBe('cancelled');
    await client.close();
    await expect(client.cancel('session-one')).rejects.toThrow('not available');
  });

  it('captures update failures without poisoning a later prompt', async () => {
    const process = new FakeAcpProcess();
    let prompts = 0;
    serve(process, (message) => {
      if (message.method === 'initialize') {
        process.send({
          jsonrpc: '2.0',
          id: message.id,
          result: { protocolVersion: 1, authMethods: [{ id: 'cursor_login' }] },
        });
      } else if (message.method === 'session/prompt') {
        prompts += 1;
        process.send(update('session-one', 'agent_message_chunk', 'delta'));
        process.send({
          jsonrpc: '2.0',
          id: message.id,
          result: { stopReason: prompts === 1 ? 'refusal' : 'max_tokens' },
        });
      }
    });
    const client = new AcpJsonRpcClient(process.asCliProcess(), 1_000, 1_000);
    await client.initialize();

    await expect(
      client.prompt('session-one', 'first', async () => {
        throw new Error('event persistence failed');
      }),
    ).rejects.toThrow('event persistence failed');
    await expect(client.prompt('session-one', 'second', async () => undefined)).resolves.toBe(
      'max_tokens',
    );
    await client.close();
  });

  it('redacts credentials in remote errors and handles process failure', async () => {
    const process = new FakeAcpProcess();
    serve(process, (message) => {
      if (message.method === 'initialize') {
        process.send({
          jsonrpc: '2.0',
          id: message.id,
          result: { protocolVersion: 1, authMethods: [{ id: 'cursor_login' }] },
        });
      } else if (message.method === 'session/new') {
        process.send({
          jsonrpc: '2.0',
          id: message.id,
          error: {
            code: -32000,
            message:
              'private@example.com accessToken=cursor-secret-token-123456 Bearer eyJabc.def.ghi\u0000',
          },
        });
      }
    });
    const client = new AcpJsonRpcClient(process.asCliProcess(), 1_000, 1_000);
    await client.initialize();
    const error = await client.newSession('/repo').catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(CursorCliError);
    expect(String(error)).not.toContain('private@example.com');
    expect(String(error)).not.toContain('cursor-secret-token');
    expect(String(error)).not.toContain('eyJabc');
    expect(String(error)).not.toContain('\u0000');
    process.emit('error', new Error('spawn failed with private environment'));
    await expect(client.cancel('session-one')).rejects.toThrow('not available');
  });

  it('enforces the configured ACP frame limit', async () => {
    const process = new FakeAcpProcess();
    serve(process, (message) => {
      if (message.method === 'initialize') {
        process.send({
          jsonrpc: '2.0',
          id: message.id,
          result: { protocolVersion: 1, authMethods: [{ id: 'cursor_login' }] },
        });
      }
    });
    const client = new AcpJsonRpcClient(process.asCliProcess(), 1_000, 1_000, 100, 256);
    await client.initialize();

    process.stdout.write('x'.repeat(257));

    await expect(client.cancel('session-one')).rejects.toThrow('not available');
    await client.close();
  });

  it('forces a resistant ACP child down within the configured close window', async () => {
    const process = new FakeAcpProcess(true);
    serve(process, (message) => {
      if (message.method === 'initialize') {
        process.send({
          jsonrpc: '2.0',
          id: message.id,
          result: { protocolVersion: 1, authMethods: [{ id: 'cursor_login' }] },
        });
      }
    });
    const client = new AcpJsonRpcClient(process.asCliProcess(), 1_000, 1_000, 20, 16 * 1024 * 1024);
    await client.initialize();

    await client.close();

    expect(process.signals).toEqual(['SIGTERM', 'SIGKILL']);
  });
});

class FakeAcpProcess extends EventEmitter {
  readonly stdin = new PassThrough();
  readonly stdout = new PassThrough();
  readonly stderr = new PassThrough();
  exitCode: number | null = null;
  readonly signals: NodeJS.Signals[] = [];
  private ended = false;

  constructor(private readonly resistSigterm = false) {
    super();
  }

  send(message: Record<string, unknown>): void {
    this.stdout.write(`${JSON.stringify(message)}\n`);
  }

  kill(signal: NodeJS.Signals = 'SIGTERM'): boolean {
    this.signals.push(signal);
    if (this.resistSigterm && signal === 'SIGTERM') return true;
    if (this.ended) return false;
    this.ended = true;
    this.exitCode = 0;
    queueMicrotask(() => {
      this.emit('exit', 0, signal);
      this.emit('close');
      this.stdout.end();
      this.stderr.end();
    });
    return true;
  }

  asCliProcess(): ConstructorParameters<typeof AcpJsonRpcClient>[0] {
    return this as never;
  }
}

function serve(process: FakeAcpProcess, handler: (message: Record<string, unknown>) => void): void {
  const lines = createInterface({ input: process.stdin, crlfDelay: Number.POSITIVE_INFINITY });
  lines.on('line', (line) => handler(JSON.parse(line) as Record<string, unknown>));
}

function update(sessionId: string, sessionUpdate: string, text: string): Record<string, unknown> {
  return {
    jsonrpc: '2.0',
    method: 'session/update',
    params: {
      sessionId,
      update: { sessionUpdate, content: { type: 'text', text } },
    },
  };
}

function requestId(message: Record<string, unknown>): number | string | null {
  return typeof message.id === 'number' || typeof message.id === 'string' ? message.id : null;
}
