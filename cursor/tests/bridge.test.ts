import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import { z } from 'zod';
import {
  BridgeProcessError,
  ConnectJsonTransport,
  ManagedBridgeClient,
  parseDiscoveryLine,
  type BridgeProcess,
} from '../src/bridge.js';

const ALL_CAPABILITIES = [
  'agent.create',
  'agent.resume',
  'agent.send',
  'run.observe',
  'run.wait',
  'run.cancel',
  'agent.management',
  'cursor.catalog',
  'agent.usage',
];

function frame(flags: number, value: unknown): Buffer {
  const body = Buffer.from(JSON.stringify(value));
  const result = Buffer.alloc(5 + body.length);
  result.writeUInt8(flags, 0);
  result.writeUInt32BE(body.length, 1);
  body.copy(result, 5);
  return result;
}

function streamResponse(chunks: Uint8Array[], status = 200): Response {
  return new Response(
    new ReadableStream({
      start(controller) {
        for (const chunk of chunks) controller.enqueue(chunk);
        controller.close();
      },
    }),
    { status },
  );
}

describe('ConnectJsonTransport', () => {
  it('sends authenticated lower-level unary requests', async () => {
    const requests: Array<{ url: string; init: RequestInit }> = [];
    const transport = new ConnectJsonTransport(
      'http://127.0.0.1:9000',
      'bridge-token',
      (async (url, init) => {
        requests.push({ url: String(url), init: init ?? {} });
        return Response.json({ message: 'pong', extra: true });
      }) as typeof fetch,
      1_000,
      1_024,
    );

    const result = await transport.unary(
      'SdkBridgeControlService',
      'Ping',
      { requestId: 'one' },
      z.object({ message: z.string() }).passthrough(),
    );

    expect(result.message).toBe('pong');
    expect(requests[0]?.url).toBe('http://127.0.0.1:9000/sdk.v1.SdkBridgeControlService/Ping');
    expect(requests[0]?.init.headers).toMatchObject({
      Authorization: 'Bearer bridge-token',
      'Connect-Protocol-Version': '1',
      'Content-Type': 'application/json',
    });
    expect(JSON.parse(Buffer.from(requests[0]?.init.body as Uint8Array).toString())).toEqual({
      requestId: 'one',
    });
  });

  it('bounds unary requests and responses', async () => {
    let calls = 0;
    const requestLimited = new ConnectJsonTransport(
      'http://localhost:1',
      'token',
      (async () => {
        calls += 1;
        return Response.json({});
      }) as typeof fetch,
      1_000,
      8,
    );
    await expect(
      requestLimited.unary('S', 'M', { value: 'too long' }, z.object({})),
    ).rejects.toThrow('request exceeds 8 bytes');
    expect(calls).toBe(0);

    const declaredOversized = new ConnectJsonTransport(
      'http://localhost:1',
      'token',
      (async () =>
        new Response('{"value":1}', {
          headers: { 'content-length': '100' },
        })) as typeof fetch,
      1_000,
      32,
    );
    await expect(
      declaredOversized.unary('S', 'M', {}, z.object({ value: z.number() })),
    ).rejects.toThrow('response exceeds 32 bytes');

    const chunkedOversized = new ConnectJsonTransport(
      'http://localhost:1',
      'token',
      (async () =>
        new Response(
          new ReadableStream({
            start(controller) {
              controller.enqueue(new TextEncoder().encode('{"value":"too long"}'));
              controller.close();
            },
          }),
        )) as typeof fetch,
      1_000,
      12,
    );
    await expect(
      chunkedOversized.unary('S', 'M', {}, z.object({ value: z.string() })),
    ).rejects.toThrow('response exceeds 12 bytes');
  });

  it('decodes fragmented stream frames and requires the end frame', async () => {
    const wire = Buffer.concat([
      frame(0, { value: 1 }),
      frame(0, { value: 2, unknown: 'kept' }),
      frame(2, {}),
    ]);
    const chunks = [...wire].map((byte) => Uint8Array.of(byte));
    let requestBody = Buffer.alloc(0);
    const transport = new ConnectJsonTransport(
      'http://localhost:9000',
      'token',
      (async (_url, init) => {
        requestBody = Buffer.from(init?.body as Uint8Array);
        return streamResponse(chunks);
      }) as typeof fetch,
      1_000,
      1_024,
    );

    const values = [];
    for await (const item of transport.stream(
      'SdkAgentService',
      'Send',
      { agentId: 'bc-one' },
      z.object({ value: z.number() }).passthrough(),
    )) {
      values.push(item);
    }

    expect(values.map((item) => item.value)).toEqual([1, 2]);
    expect(requestBody.readUInt8(0)).toBe(0);
    expect(requestBody.readUInt32BE(1)).toBe(requestBody.length - 5);
    expect(JSON.parse(requestBody.subarray(5).toString())).toEqual({ agentId: 'bc-one' });
  });

  it('rejects compressed, oversized, and prematurely closed streams', async () => {
    const schema = z.object({ value: z.number() });
    const compressed = new ConnectJsonTransport(
      'http://localhost:1',
      'token',
      (async () => streamResponse([frame(1, { value: 1 }), frame(2, {})])) as typeof fetch,
      1_000,
      1_024,
    );
    await expect(collect(compressed.stream('S', 'M', {}, schema))).rejects.toThrow(
      'Compressed Connect frames are not supported',
    );

    const oversizedHeader = Buffer.alloc(5);
    oversizedHeader.writeUInt32BE(1_025, 1);
    const oversized = new ConnectJsonTransport(
      'http://localhost:1',
      'token',
      (async () => streamResponse([oversizedHeader])) as typeof fetch,
      1_000,
      1_024,
    );
    await expect(collect(oversized.stream('S', 'M', {}, schema))).rejects.toThrow(
      'exceeds 1024 bytes',
    );

    const truncated = new ConnectJsonTransport(
      'http://localhost:1',
      'token',
      (async () => streamResponse([frame(0, { value: 1 })])) as typeof fetch,
      1_000,
      1_024,
    );
    await expect(collect(truncated.stream('S', 'M', {}, schema))).rejects.toThrow(
      'ended before EndStreamResponse',
    );
  });

  it('surfaces an error from an EndStreamResponse', async () => {
    const transport = new ConnectJsonTransport(
      'http://localhost:1',
      'token',
      (async () =>
        streamResponse([
          frame(2, { error: { code: 'resource_exhausted', message: 'try later' } }),
        ])) as typeof fetch,
      1_000,
      1_024,
    );

    await expect(
      collect(transport.stream('S', 'M', {}, z.object({}).passthrough())),
    ).rejects.toMatchObject({
      code: 'resource_exhausted',
      message: 'resource_exhausted: try later',
    });
  });

  it('times out each idle stream read without imposing an overall deadline', async () => {
    const transport = new ConnectJsonTransport(
      'http://localhost:1',
      'token',
      (async () => new Response(new ReadableStream())) as typeof fetch,
      1_000,
      1_024,
    );

    await expect(
      collect(transport.stream('S', 'M', {}, z.object({}), { timeoutMs: 10 })),
    ).rejects.toThrow('stream idle timeout after 10ms');
  });
});

describe('ManagedBridgeClient', () => {
  it('starts lazily, reads the token file, validates capabilities, and shuts down', async () => {
    const process = new FakeProcess();
    const spawned: Array<Record<string, unknown>> = [];
    const calls: Array<{ url: string; authorization: string | null }> = [];
    const client = new ManagedBridgeClient(
      {
        binary: '/opt/cursor-sdk-bridge',
        workspace: '/repo',
        apiKey: 'key_secret',
        startupTimeoutMs: 1_000,
        shutdownTimeoutMs: 20,
        rpcTimeoutMs: 1_000,
        maxFrameBytes: 4_096,
      },
      {
        spawn: (command, args, options) => {
          spawned.push({ command, args, env: options.env });
          queueMicrotask(() => {
            process.stderr.write(
              'cursor-sdk-bridge ready {"schemaVersion":1,"transport":"tcp","protocol":"connect","url":"http://127.0.0.1:7777","authTokenFile":"/tmp/token"}\n',
            );
          });
          return process;
        },
        readFile: async () => ' bridge-token\n',
        fetch: (async (url, init) => {
          const headers = new Headers(init?.headers);
          calls.push({ url: String(url), authorization: headers.get('authorization') });
          if (String(url).endsWith('/Ping')) return Response.json({ message: 'pong' });
          if (String(url).endsWith('/GetVersion')) {
            return Response.json({
              bridgeVersion: '1.0.0',
              protocolVersion: 'sdk.v1',
              capabilities: ALL_CAPABILITIES,
            });
          }
          if (String(url).endsWith('/Shutdown')) {
            process.exitCode = 0;
            process.emit('exit', 0, null);
            return Response.json({});
          }
          return Response.json({ items: [] });
        }) as typeof fetch,
      },
    );

    expect(spawned).toHaveLength(0);
    await client.unary(
      'SdkCursorService',
      'ListModels',
      { options: { apiKey: 'key_secret' } },
      z.object({ items: z.array(z.unknown()) }),
    );
    await client.close();

    expect(spawned).toHaveLength(1);
    expect(spawned[0]?.command).toBe('/opt/cursor-sdk-bridge');
    expect(spawned[0]?.args).toEqual(['--workspace', '/repo']);
    expect((spawned[0]?.env as NodeJS.ProcessEnv).CURSOR_SDK_CLIENT_LANGUAGE).toBe('node');
    expect((spawned[0]?.env as NodeJS.ProcessEnv).CURSOR_API_KEY).toBe('key_secret');
    expect(calls.every((call) => call.authorization === 'Bearer bridge-token')).toBe(true);
    expect(calls.some((call) => call.url.endsWith('/Shutdown'))).toBe(true);
  });

  it('rejects non-loopback discovery without exposing the API key', async () => {
    const process = new FakeProcess();
    const secret = 'key_do_not_print';
    const client = new ManagedBridgeClient(
      {
        binary: 'cursor-sdk-bridge',
        workspace: '.',
        apiKey: secret,
        startupTimeoutMs: 100,
        shutdownTimeoutMs: 1,
        rpcTimeoutMs: 100,
        maxFrameBytes: 1_024,
      },
      {
        spawn: () => {
          queueMicrotask(() => {
            process.stderr.write(
              'cursor-sdk-bridge ready {"schemaVersion":1,"transport":"tcp","protocol":"connect","url":"https://example.com","authToken":"secret"}\n',
            );
          });
          return process;
        },
        readFile: async () => '',
        fetch,
      },
    );

    await expect(
      client.unary('SdkBridgeControlService', 'Ping', {}, z.object({ message: z.string() })),
    ).rejects.toSatisfy((error: unknown) => {
      expect(error).toBeInstanceOf(BridgeProcessError);
      expect(String(error)).not.toContain(secret);
      return true;
    });
    expect(process.stderr.listenerCount('data')).toBe(0);
  });

  it.each([
    ['a non-pong Ping', { message: 'not-pong' }, null],
    [
      'a different protocol',
      { message: 'pong' },
      { bridgeVersion: '1.0.0', protocolVersion: 'sdk.v2', capabilities: ALL_CAPABILITIES },
    ],
    [
      'missing capabilities',
      { message: 'pong' },
      { bridgeVersion: '1.0.0', protocolVersion: 'sdk.v1', capabilities: ['agent.create'] },
    ],
  ])('rejects startup with %s', async (_name, ping, version) => {
    const process = new FakeProcess();
    const client = managedClient(process, (url) => {
      if (url.endsWith('/Ping')) return Response.json(ping);
      if (url.endsWith('/GetVersion')) return Response.json(version);
      return Response.json({});
    });

    await expect(
      client.unary('SdkCursorService', 'ListModels', {}, z.object({})),
    ).rejects.toBeInstanceOf(BridgeProcessError);
    expect(process.signals[0]).toBe('SIGTERM');
  });

  it('redacts discovery validation failures and can retry after startup rejection', async () => {
    const processes = [new FakeProcess(), new FakeProcess()];
    let spawns = 0;
    const client = new ManagedBridgeClient(launchOptions(), {
      spawn: () => {
        const process = processes[spawns++];
        if (!process) throw new Error('unexpected spawn');
        queueMicrotask(() => {
          process.stderr.write(
            spawns === 1
              ? 'cursor-sdk-bridge ready {"schemaVersion":"token_private_123","transport":"tcp","protocol":"connect","url":"http://127.0.0.1:7777","authToken":"bridge"}\n'
              : readyLine(),
          );
        });
        return process;
      },
      readFile: async () => 'bridge-token',
      fetch: (async (url) => {
        if (String(url).endsWith('/Ping')) return Response.json({ message: 'pong' });
        if (String(url).endsWith('/GetVersion')) {
          return Response.json({
            bridgeVersion: '1.0.0',
            protocolVersion: 'sdk.v1',
            capabilities: ALL_CAPABILITIES,
          });
        }
        return Response.json({});
      }) as typeof fetch,
    });

    const error = await client
      .unary('SdkCursorService', 'ListModels', {}, z.object({}))
      .catch((caught: unknown) => caught);
    expect(error).toBeInstanceOf(Error);
    expect(String(error)).not.toContain('token_private_123');
    await expect(client.unary('SdkCursorService', 'ListModels', {}, z.object({}))).resolves.toEqual(
      {},
    );
    expect(spawns).toBe(2);
    await client.close();
  });
});

it('parses only the documented ready-line prefix', () => {
  expect(parseDiscoveryLine('diagnostic')).toBeNull();
  expect(
    parseDiscoveryLine(
      'cursor-sdk-bridge ready {"schemaVersion":1,"transport":"tcp","protocol":"connect","host":"127.0.0.1","port":1}',
    ),
  ).toMatchObject({ schemaVersion: 1, port: 1 });
  expect(() => parseDiscoveryLine('cursor-sdk-bridge ready nope')).toThrow(BridgeProcessError);
});

async function collect<T>(source: AsyncIterable<T>): Promise<T[]> {
  const items: T[] = [];
  for await (const item of source) items.push(item);
  return items;
}

class FakeProcess extends EventEmitter implements BridgeProcess {
  readonly stderr = new PassThrough();
  readonly signals: Array<NodeJS.Signals | number> = [];
  exitCode: number | null = null;
  pid = 123;

  kill(signal: NodeJS.Signals | number = 'SIGTERM'): boolean {
    this.signals.push(signal);
    this.exitCode = 0;
    this.emit('exit', 0, null);
    return true;
  }
}

function launchOptions() {
  return {
    binary: '/opt/cursor-sdk-bridge',
    workspace: '/repo',
    apiKey: 'key_secret',
    startupTimeoutMs: 1_000,
    shutdownTimeoutMs: 20,
    rpcTimeoutMs: 1_000,
    maxFrameBytes: 4_096,
  };
}

function readyLine(): string {
  return 'cursor-sdk-bridge ready {"schemaVersion":1,"transport":"tcp","protocol":"connect","url":"http://127.0.0.1:7777","authTokenFile":"/tmp/token"}\n';
}

function managedClient(process: FakeProcess, respond: (url: string) => Response) {
  return new ManagedBridgeClient(launchOptions(), {
    spawn: () => {
      queueMicrotask(() => process.stderr.write(readyLine()));
      return process;
    },
    readFile: async () => 'bridge-token',
    fetch: ((url) => Promise.resolve(respond(String(url)))) as typeof fetch,
  });
}
