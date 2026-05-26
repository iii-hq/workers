import { useState } from 'react'
import { Prompt } from '@/components/ui/Prompt'
import { CodeHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import { langFromRunLang, pillForExit, truncateMiddle } from './format'
import {
  type RunRequest,
  type RunResponse,
  runRequestSchema,
  runResponseSchema,
  safeParseResponse,
} from './parsers'
import { AnsiOutput } from './terminal/AnsiOutput'
import { Chip, FooterPill, Terminal } from './terminal/Terminal'

interface RunViewProps {
  input: unknown
  output: unknown
  running?: boolean
}

export function RunView({ input, output, running }: RunViewProps) {
  const req = runRequestSchema.safeParse(input)
  if (!req.success) return null
  const respData =
    output != null ? safeParseResponse(runResponseSchema, output) : null
  return (
    <Terminal
      command={`${interpreterFor(req.data.lang)} /tmp/run.${extFor(req.data.lang)}`}
      running={running}
      chips={<RunChips req={req.data} resp={respData} />}
      footer={respData ? <RunFooter resp={respData} /> : null}
    >
      <CodePreview req={req.data} />
      <AnsiOutput stdout={respData?.stdout} stderr={respData?.stderr} />
    </Terminal>
  )
}

export function RunPreview({ input }: { input: unknown }) {
  const req = runRequestSchema.safeParse(input)
  if (!req.success) return null
  return (
    <div className="border-t border-rule-2 bg-paper-2 px-3 py-2 flex flex-wrap items-center gap-x-3 gap-y-1.5">
      <span className="font-mono text-[12.5px] text-ink">
        <Prompt symbol="$" />
        <span className="ml-2">{`run ${req.data.lang} /tmp/run.${extFor(req.data.lang)}`}</span>
      </span>
      <span className="flex flex-wrap items-center gap-1.5 ml-auto">
        <RunChips req={req.data} resp={null} />
      </span>
    </div>
  )
}

function CodePreview({ req }: { req: RunRequest }) {
  const [open, setOpen] = useState(false)
  const lang = langFromRunLang(req.lang)
  const lineCount = req.code.split('\n').length
  return (
    <div className="border-b border-rule-2">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className={cn(
          'w-full bg-paper-2 px-3 py-1.5 border-b border-rule-2 flex items-center gap-2',
          'font-mono text-[11px] uppercase tracking-[0.06em] text-ink-faint',
          'hover:text-ink transition-colors text-left cursor-pointer',
        )}
      >
        <span
          aria-hidden
          className={cn(
            'inline-block transition-transform',
            open && 'rotate-90',
          )}
        >
          ▸
        </span>
        source
        <span className="text-ink-ghost normal-case tracking-normal">
          · /tmp/run.{extFor(req.lang)} · {lineCount}{' '}
          {lineCount === 1 ? 'line' : 'lines'}
        </span>
      </button>
      {open ? (
        lang ? (
          <CodeHighlight code={req.code} language={lang} wrap />
        ) : (
          <pre className="bg-bg overflow-x-auto px-3 py-2 font-mono text-[12.5px] leading-[1.55] text-ink whitespace-pre-wrap break-words">
            <code>{req.code}</code>
          </pre>
        )
      ) : null}
    </div>
  )
}

function RunChips({
  req,
  resp,
}: {
  req: RunRequest
  resp: RunResponse | null
}) {
  return (
    <>
      <Chip label="image">{req.image}</Chip>
      <Chip label="lang">{req.lang}</Chip>
      {req.keep_sandbox ? <Chip label="keep">{'true'}</Chip> : null}
      {resp?.sandbox_id ? (
        <Chip label="sandbox">{truncateMiddle(resp.sandbox_id, 12)}</Chip>
      ) : null}
    </>
  )
}

function RunFooter({ resp }: { resp: RunResponse }) {
  const exit = pillForExit(resp.exit_code)
  return (
    <>
      <FooterPill tone={exit.tone}>{exit.label}</FooterPill>
      <FooterPill>{`${resp.duration_ms}ms`}</FooterPill>
      {resp.timed_out ? <FooterPill tone="warn">timed out</FooterPill> : null}
      {resp.sandbox_id ? (
        <FooterPill tone="warn">kept alive</FooterPill>
      ) : (
        <FooterPill tone="default">auto-stopped</FooterPill>
      )}
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
