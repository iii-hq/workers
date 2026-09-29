/**
 * `sandbox::run` — the terminal card with a collapsible source pane.
 * Ported from the console's RunView; syntax highlighting comes from
 * the console-injected `CodeHighlight`, the footer verdict is the
 * single exit-reason pill, and a kept sandbox's id chip jumps to the
 * fleet page.
 */

import { Badge, CodeHighlight, TerminalCommandLine } from '@iii-dev/console-ui'
import { uiClasses } from '@iii-dev/console-ui/ui-classes'
import { ChevronRight } from 'lucide-react'
import { useState } from 'react'
import { exitReason, langFromRunLang } from './format'
import { type RunRequest, type RunResponse, runRequestSchema, runResponseSchema, safeParseResponse } from './parsers'
import { Chip, cx, ExitReasonPill, SandboxIdChip, Streams, Terminal } from './shared'

interface RunViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function RunView({ input, output, running }: RunViewProps) {
  const req = runRequestSchema.safeParse(input)
  if (!req.success) return null
  const respData = output != null ? safeParseResponse(runResponseSchema, output) : null
  return (
    <Terminal
      command={`${interpreterFor(req.data.lang)} /tmp/run.${extFor(req.data.lang)}`}
      running={running}
      chips={<RunChips req={req.data} resp={respData} />}
      footer={respData ? <RunFooter resp={respData} /> : null}
    >
      <CodePreview req={req.data} />
      <Streams stdout={respData?.stdout} stderr={respData?.stderr} />
    </Terminal>
  )
}

export function RunPreview({ input }: { input: unknown }) {
  const req = runRequestSchema.safeParse(input)
  if (!req.success) return null
  return (
    <div className="cr-fam-card">
      <TerminalCommandLine
        command={`run ${req.data.lang} /tmp/run.${extFor(req.data.lang)}`}
        chips={<RunChips req={req.data} resp={null} />}
        className="cr-fam-term-head"
      />
    </div>
  )
}

function CodePreview({ req }: { req: RunRequest }) {
  const [open, setOpen] = useState(false)
  const lang = langFromRunLang(req.lang)
  const lineCount = req.code.split('\n').length
  return (
    <div className="cr-fam-src">
      <button type="button" onClick={() => setOpen((v) => !v)} aria-expanded={open} className="cr-fam-src-toggle">
        <ChevronRight size={16} aria-hidden className={cx('cr-fam-caret', open && 'open')} />
        <span className={uiClasses.eyebrow}>source</span>
        <span className="cr-fam-src-meta">
          · /tmp/run.{extFor(req.lang)} · {lineCount} {lineCount === 1 ? 'line' : 'lines'}
        </span>
      </button>
      {open ? (
        <div className="cr-fam-code">
          <CodeHighlight code={req.code} language={lang ?? 'text'} wrap />
        </div>
      ) : null}
    </div>
  )
}

function RunChips({ req, resp }: { req: RunRequest; resp: RunResponse | null }) {
  return (
    <>
      <Chip label="image">{req.image}</Chip>
      <Chip label="lang">{req.lang}</Chip>
      {req.keep_sandbox ? <Chip label="keep">{'true'}</Chip> : null}
      {resp?.sandbox_id ? <SandboxIdChip sandboxId={resp.sandbox_id} /> : null}
    </>
  )
}

function RunFooter({ resp }: { resp: RunResponse }) {
  return (
    <>
      <ExitReasonPill reason={exitReason(resp)} />
      <Badge>{`${resp.duration_ms}ms`}</Badge>
      {resp.sandbox_id ? <Badge variant="warn">kept alive</Badge> : <Badge>auto-stopped</Badge>}
    </>
  )
}

function interpreterFor(lang: string): string {
  const l = lang.toLowerCase()
  if (l === 'node' || l === 'js' || l === 'javascript') return 'node'
  if (l === 'python' || l === 'py') return 'python3'
  if (l === 'shell' || l === 'sh' || l === 'bash') return '/bin/sh'
  return lang
}

function extFor(lang: string): string {
  const l = lang.toLowerCase()
  if (l === 'node' || l === 'js' || l === 'javascript') return 'js'
  if (l === 'python' || l === 'py') return 'py'
  if (l === 'shell' || l === 'sh' || l === 'bash') return 'sh'
  return 'txt'
}
