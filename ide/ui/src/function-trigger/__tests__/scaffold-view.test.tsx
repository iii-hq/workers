import type { FunctionTriggerMessage, Host } from '@iii-dev/console-ui'
import type { ReactNode } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it, vi } from 'vitest'

vi.mock('@iii-dev/console-ui', () => {
  const tag =
    (name: string) =>
    ({ children, ...props }: { children?: ReactNode; [key: string]: unknown }) => (
      <span data-c={name} data-variant={props.variant as string | undefined}>
        {children}
      </span>
    )
  return {
    Badge: tag('Badge'),
    Button: ({ children }: { children?: ReactNode }) => <button type="button">{children}</button>,
    Chip: tag('Chip'),
    EmptyState: ({ title, description }: { title: string; description?: string }) => (
      <div data-c="EmptyState">
        {title} {description}
      </div>
    ),
    Eyebrow: tag('Eyebrow'),
    IconButton: ({ label }: { label: string }) => <button type="button" aria-label={label} />,
    StatusDot: ({ tone }: { tone: string }) => <i data-c="StatusDot" data-tone={tone} />,
    StatusPanel: ({ headline, detail }: { headline: ReactNode; detail?: ReactNode }) => (
      <div data-c="StatusPanel">
        {headline} {detail}
      </div>
    ),
    TerminalStream: tag('TerminalStream'),
    uiClasses: { pulse: 'pulse' },
  }
})

import { StartFailure } from '../../page/worker-result'
import { createScaffoldRenderer, harnessNote, notStartedReason, scaffoldRequest, shortFolder } from '../ScaffoldView'

const host = { iii: { trigger: vi.fn(() => new Promise(() => {})) }, panels: { open: vi.fn() } } as unknown as Host
const renderer = createScaffoldRenderer(host)

function message(functionId: string, extra: Partial<FunctionTriggerMessage> = {}): FunctionTriggerMessage {
  return { id: 'm1', role: 'function-trigger', functionId, input: {}, createdAt: 0, ...extra }
}

const html = (node: ReactNode) => renderToStaticMarkup(<>{node}</>)

/** The harness envelope the console receives a result in. */
const envelope = (details: unknown, ...notes: string[]) => ({
  content: [{ type: 'text', text: JSON.stringify(details) }, ...notes.map((text) => ({ type: 'text', text }))],
  details,
  terminate: false,
})

const SCAFFOLD = {
  template: 'worker-node-ade',
  name: 'todo-a7',
  directory: '/repo/workers/todo-a7',
  files: [
    { path: '/repo/workers/todo-a7/package.json', bytes: 10, revision: 'r1' },
    { path: '/repo/workers/todo-a7/src/index.ts', bytes: 20, revision: 'r2' },
  ],
  compose: { worker: '/repo/workers/todo-a7' },
  compose_add: { workers: [] },
  requires: ['http'],
  next_steps: [],
}

describe('scaffold renderer', () => {
  it('claims only the two template functions and leaves approvals to the host', () => {
    expect(renderer.isMatch('coder::list-templates')).toBe(true)
    expect(renderer.isMatch('coder::scaffold-worker')).toBe(true)
    expect(renderer.isMatch('coder::create-file')).toBe(false)
    expect(renderer.tryRender(message('coder::scaffold-worker', { pendingApproval: true }))).toBeNull()
  })

  it('lists the templates with their source, language, page and requirements', () => {
    const out = html(
      renderer.tryRender(
        message('coder::list-templates', {
          output: envelope({
            source: {
              kind: 'git',
              location: 'https://github.com/iii-hq/templates.git',
              ref: 'main',
              revision: 'abcdef123',
            },
            templates: [
              {
                id: 'worker-node-ade',
                name: 'Worker with ADE page (Node)',
                description: 'A Node worker with a public page.',
                language: 'node',
                requires: ['http'],
              },
              { id: 'worker-python', name: 'Worker (Python)', description: '', language: 'python', requires: [] },
            ],
          }),
        }),
      ),
    )
    expect(out).toContain('2 worker templates')
    expect(out).toContain('iii-hq/templates@main · abcdef1')
    expect(out).toContain('Worker with ADE page (Node)')
    expect(out).toContain('worker-node-ade')
    expect(out).toContain('>Node<')
    expect(out).toContain('ADE page')
    expect(out).toContain('needs http')
    expect(out).toContain('New worker')
  })

  it('warns when the template source is stale, and says when it has none', () => {
    const out = html(
      renderer.tryRender(
        message('coder::list-templates', {
          output: { source: { kind: 'dir', location: '/t', warning: 'fetch failed; using the cache' }, templates: [] },
        }),
      ),
    )
    expect(out).toContain('local: /t')
    expect(out).toContain('fetch failed; using the cache')
    expect(out).toContain('No worker templates')
    expect(out).not.toContain('New worker')
  })

  it('shows a call in flight as files being written, then install and start', () => {
    const out = html(
      renderer.tryRenderRunning?.(
        message('coder::scaffold-worker', { running: true, input: { name: 'todo-a7', template: 'worker-node-ade' } }),
      ),
    )
    expect(out).toContain('todo-a7')
    expect(out).toContain('worker-node-ade → workers/todo-a7')
    expect(out).toContain('Writing files…')
    expect(out).toContain('Install')
    expect(out).toContain('Start')
  })

  it('follows a started worker from its files to installing', () => {
    const out = html(
      renderer.tryRender(
        message('coder::scaffold-worker', {
          input: { name: 'todo-a7' },
          output: envelope({ ...SCAFFOLD, operation_id: 'add-todo-a7', started: ['todo-a7', 'http'] }),
        }),
      ),
    )
    expect(out).toContain('Created 2 files')
    expect(out).toContain('src/index.ts')
    expect(out).toContain('Installing…')
    expect(out).toContain('worker-node-ade → …/workers/todo-a7')
  })

  it("says why only the files were written, without the agent's next call", () => {
    const note =
      '[harness] Not started (approval holds compose::add): only the files were written. Add it with compose::add, sending compose_add whole.'
    const out = html(renderer.tryRender(message('coder::scaffold-worker', { output: envelope(SCAFFOLD, note) })))
    expect(out).toContain('Not added to the stack')
    expect(out).toContain('Approval holds compose::add.')
    expect(out).toContain('ask the agent to add it to the stack')
    expect(out).not.toContain('compose_add whole')
    expect(out).not.toContain('Installing')
    expect(harnessNote(envelope(SCAFFOLD))).toBeNull()
  })

  it('marks a start compose::add refused as failed, with its error', () => {
    const out = html(
      renderer.tryRender(
        message('coder::scaffold-worker', { output: envelope({ ...SCAFFOLD, start_error: 'compose is not running' }) }),
      ),
    )
    expect(out).toContain('Not added to the stack')
    expect(out).toContain('compose is not running')
  })

  it('shows the approver what the call writes and whether it starts the worker', () => {
    const starts = html(
      renderer.tryRenderPreview?.(
        message('coder::scaffold-worker', { input: { name: 'todo-a7', template: 'worker-node-ade' } }),
      ),
    )
    expect(starts).toContain('Create todo-a7')
    expect(starts).toContain('Write files into')
    expect(starts).toContain('workers/todo-a7')
    expect(starts).toContain('compose::add')
    expect(starts).toContain('Start')
    const filesOnly = html(
      renderer.tryRenderPreview?.(message('coder::scaffold-worker', { input: { name: 'x', start: false } })),
    )
    expect(filesOnly).toContain('files only')
    expect(renderer.tryRenderPreview?.(message('coder::list-templates'))).toBeNull()
  })

  it('reads the request through the envelope and ignores fields of the wrong type', () => {
    expect(scaffoldRequest({ name: 'a', template: 7, start: 'yes' })).toEqual({
      name: 'a',
      template: undefined,
      directory: undefined,
      start: undefined,
    })
  })

  it('keeps the end of a long folder, where the worker name is', () => {
    expect(shortFolder('/home/me/project/workers/todo-a7')).toBe('…/workers/todo-a7')
    expect(shortFolder('workers/todo-a7')).toBe('workers/todo-a7')
  })

  it("keeps only the harness note's reason, as a sentence", () => {
    expect(notStartedReason('Not started (the approval gate holds compose::add): only the files.')).toBe(
      'The approval gate holds compose::add.',
    )
    expect(notStartedReason('something else')).toBeNull()
    expect(notStartedReason(null)).toBeNull()
  })

  it('sets backticked commands in a failure as code, and keeps an unmatched backtick', () => {
    const out = html(<StartFailure error="pre_run `pnpm install` exited with status 1" logs={[]} />)
    expect(out).toContain('<code class="shui-new-worker-code">pnpm install</code>')
    expect(out).toContain('class="shui-new-worker-note alert"')
    expect(html(<StartFailure error="a `b" logs={[]} />)).toContain('a `b')
  })
})
