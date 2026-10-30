import { Check, Copy } from 'lucide-react'
import { useState } from 'react'
import { copyTextToClipboard } from '@/lib/clipboard'
import { CodeHighlight } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import './code-block.css'

/** Fences whose lines are commands a reader will paste into a terminal. */
const SHELL_LANGUAGES = new Set([
  'bash',
  'sh',
  'shell',
  'zsh',
  'console',
  'terminal',
])

/** Prism ids for the aliases people actually type after a fence. */
const LANGUAGE_ALIASES: Record<string, string> = {
  console: 'bash',
  terminal: 'bash',
  js: 'javascript',
  jsx: 'jsx',
  ts: 'typescript',
  tsx: 'tsx',
  py: 'python',
  rs: 'rust',
  yml: 'yaml',
  md: 'markdown',
}

export function isShellLanguage(language: string | undefined): boolean {
  return language !== undefined && SHELL_LANGUAGES.has(language)
}

/**
 * Which shell lines read as commands: anything that is not blank, not a
 * comment, not an indented continuation, and not the tail of a `\`
 * line-continuation. Those get the `$` prompt; the rest stay bare.
 */
export function shellPromptLines(code: string): boolean[] {
  const lines = code.split('\n')
  return lines.map((line, index) => {
    if (line.trim().length === 0) return false
    if (/^\s/.test(line)) return false
    if (line.trimStart().startsWith('#')) return false
    const previous = index > 0 ? lines[index - 1] : ''
    if (/\\\s*$/.test(previous)) return false
    return true
  })
}

interface CodeBlockProps {
  code: string
  /** The fence's language id as written, lower-cased; undefined for a bare fence. */
  language?: string
  className?: string
}

/**
 * A fenced code block as a card: a 36px header naming the language with a
 * copy affordance at its trailing edge, and the highlighted source beneath
 * on the editor code surface. Shell fences prefix each command line with a
 * `$` prompt drawn as a pseudo-element, so the reader can tell commands
 * from output while selection and the copy button only ever carry the
 * source itself.
 */
export function CodeBlock({ code, language, className }: CodeBlockProps) {
  const shell = isShellLanguage(language)
  const prismLanguage = language
    ? (LANGUAGE_ALIASES[language] ?? language)
    : 'text'
  const prompts = shell ? shellPromptLines(code) : undefined
  return (
    <div
      className={cn('chat-code-block code-theme-editor', className)}
      data-code-block=""
      data-language={language ?? undefined}
      data-shell={shell || undefined}
    >
      <div className="chat-code-block__head">
        <span className="chat-code-block__lang">
          {language ?? (shell ? 'shell' : '')}
        </span>
        <CodeCopyButton code={code} />
      </div>
      <CodeHighlight
        code={code}
        language={prismLanguage}
        className="chat-code-block__pre"
        linePrompt={prompts ? (_line, index) => prompts[index] : undefined}
      />
    </div>
  )
}

/**
 * Copy → copied is a crossfade of two icons in one cell (scale, blur and
 * opacity, 200ms on the standard curve), with a scale-down on press so the
 * button acknowledges the click before the clipboard does.
 */
function CodeCopyButton({ code }: { code: string }) {
  const [copied, setCopied] = useState(false)
  const copy = () => {
    void copyTextToClipboard(code).then((ok) => {
      if (!ok) return
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1400)
    })
  }
  return (
    <button
      type="button"
      onClick={copy}
      className="chat-code-block__copy"
      data-copied={copied || undefined}
      aria-label={copied ? 'Copied' : 'Copy code'}
      title={copied ? 'Copied' : 'Copy'}
    >
      <span className="chat-code-block__copy-icons" aria-hidden="true">
        <Copy className="size-4" data-icon="copy" />
        <Check className="size-4 text-ok" data-icon="check" />
      </span>
      <span className="chat-code-block__copy-label">
        {copied ? 'Copied' : 'Copy'}
      </span>
    </button>
  )
}
