/* The commit box under the change tree: Amend, recent messages, Generate
   (with a menu naming the model and instructions it uses), the message,
   a line summing up what is ticked, and Commit / Commit and push with the
   commit options (sign-off, Git hooks, author) behind the gear. */

import type { Host } from '@iii-dev/console-ui'
import {
  Button,
  Checkbox,
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  IconButton,
  StatusDot,
  StatusPanel,
  WorkerConfigurationDialog,
} from '@iii-dev/console-ui'
import {
  Check,
  ChevronDown,
  CircleAlert,
  History,
  Settings,
  SlidersHorizontal,
  Sparkles,
  Square,
  TriangleAlert,
} from 'lucide-react'
import { useState } from 'react'
import { changeSummary } from './commit-tree'
import { gitHeadMessage, gitRecentMessages } from './git-log'
import { TextDialog } from './TextDialog'
import { modelLabel, useCommitMessage } from './use-commit-message'
import type { SourceControlState } from './use-source-control'

/** The subject length git tooling and most reviewers expect. */
const SUBJECT_LIMIT = 72
/** The configuration entry the Generate settings live in. */
const CONFIGURATION_ID = 'ide'

interface CommitBoxProps {
  host: Host
  root: string | null
  conversationId?: string | null
  scm: SourceControlState
}

export function CommitBox({ host, root, conversationId, scm }: CommitBoxProps) {
  const [message, setMessage] = useState('')
  const [amend, setAmend] = useState(false)
  const [amendSeed, setAmendSeed] = useState<string | null>(null)
  const [signOff, setSignOff] = useState(false)
  const [runHooks, setRunHooks] = useState(true)
  const [author, setAuthor] = useState('')
  const [authorOpen, setAuthorOpen] = useState(false)
  const [recent, setRecent] = useState<string[] | null>(null)
  const [replacedDraft, setReplacedDraft] = useState<string | null>(null)
  const [settingsOpen, setSettingsOpen] = useState(false)
  const gen = useCommitMessage(host, root, conversationId)

  // A branch switch starts a fresh message (adjusted during render).
  const [branchSeen, setBranchSeen] = useState(scm.branch)
  if (branchSeen !== scm.branch) {
    setBranchSeen(scm.branch)
    setMessage('')
    setAmend(false)
  }

  const writing = gen.state.phase === 'writing'
  const ticked = scm.included
  const summary = changeSummary(ticked)
  const subject = message.split('\n')[0]
  const customOptions = signOff || !runHooks || author.trim() !== ''
  const canCommit = !scm.busy && !writing && message.trim() !== '' && (ticked.length > 0 || amend)

  const toggleAmend = async (next: boolean) => {
    setAmend(next)
    if (next && message.trim() === '' && root !== null) {
      const seed = await gitHeadMessage(host, root)
      if (seed !== null) {
        setMessage(seed)
        setAmendSeed(seed)
      }
    } else if (!next && amendSeed !== null && message === amendSeed) {
      setMessage('')
      setAmendSeed(null)
    }
  }

  const generate = async () => {
    const draft = message
    const out = await gen.generate(ticked)
    if (out === null) return
    setReplacedDraft(draft.trim() !== '' && draft !== out.message ? draft : null)
    setMessage(out.message)
  }

  const submit = async (push: boolean) => {
    if (!canCommit) return
    const ok = await scm.commit({ message, amend, signOff, noVerify: !runHooks, author, push })
    if (ok) {
      setMessage('')
      setAmend(false)
      setAmendSeed(null)
      setReplacedDraft(null)
    }
  }

  const modelText = gen.model ? modelLabel(gen.model) : 'No model available'
  const generateTitle =
    ticked.length === 0
      ? 'Tick the changes to describe'
      : `Write a message for ${summary}${gen.model ? ` with ${modelLabel(gen.model)}${gen.usesChatDefault ? ' (chat default)' : ''}` : ''}`

  return (
    <form
      className="shui-commit-box"
      aria-label="Commit"
      onSubmit={(event) => {
        event.preventDefault()
        void submit(false)
      }}
    >
      <div className="shui-commit-bar">
        <Checkbox
          label="Amend"
          checked={amend}
          disabled={scm.busy || writing}
          onChange={(event) => void toggleAmend(event.currentTarget.checked)}
        />
        <DropdownMenu
          onOpenChange={(open) => {
            if (open && root !== null) void gitRecentMessages(host, root).then(setRecent)
          }}
        >
          <DropdownMenuTrigger asChild>
            <IconButton label="Recent messages" disabled={writing}>
              <History aria-hidden />
            </IconButton>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="start" className="shui-commit-menu">
            <DropdownMenuLabel>Recent messages</DropdownMenuLabel>
            {recent === null ? (
              <DropdownMenuItem disabled>Reading history…</DropdownMenuItem>
            ) : recent.length === 0 ? (
              <DropdownMenuItem disabled>No commits of yours yet</DropdownMenuItem>
            ) : (
              recent.map((text) => (
                <DropdownMenuItem key={text} title={text} onSelect={() => setMessage(text)}>
                  <span className="shui-commit-menu-text">{text.split('\n')[0]}</span>
                </DropdownMenuItem>
              ))
            )}
          </DropdownMenuContent>
        </DropdownMenu>
        <span className="spacer" />
        {writing ? (
          <Button type="button" variant="ghost" size="sm" className="shui-generate stop" onClick={gen.stop}>
            <Square aria-hidden />
            Stop
          </Button>
        ) : (
          <span className="shui-generate-split">
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="shui-generate"
              disabled={ticked.length === 0 || scm.busy}
              title={generateTitle}
              onClick={() => void generate()}
            >
              <Sparkles aria-hidden />
              Generate
            </Button>
            <DropdownMenu onOpenChange={(open) => (open ? gen.loadConfig() : undefined)}>
              <DropdownMenuTrigger asChild>
                <IconButton label="Generate options" className="shui-generate-more">
                  <ChevronDown aria-hidden />
                </IconButton>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="shui-commit-menu">
                <DropdownMenuLabel>Model</DropdownMenuLabel>
                <div className="shui-generate-model">
                  <span>{modelText}</span>
                  <span className="meta">
                    {gen.model
                      ? `${gen.usesChatDefault ? 'chat default · ' : ''}thinking ${gen.config?.thinking ?? 'default'}`
                      : 'set one in Settings'}
                  </span>
                </div>
                <DropdownMenuLabel>Instructions</DropdownMenuLabel>
                <p className="shui-generate-instructions">
                  {gen.config?.instructions.trim() || 'None. Add some in Settings, e.g. "Use Conventional Commits".'}
                </p>
                <DropdownMenuSeparator />
                <DropdownMenuItem onSelect={() => setSettingsOpen(true)}>
                  <SlidersHorizontal aria-hidden />
                  Commit message settings…
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </span>
        )}
      </div>

      {gen.state.phase === 'error' ? (
        <StatusPanel
          variant={gen.state.noModel ? 'warn' : 'alert'}
          icon={gen.state.noModel ? <TriangleAlert aria-hidden /> : <CircleAlert aria-hidden />}
          headline={gen.state.noModel ? 'No model available' : "Couldn't write a message"}
          detail={
            gen.state.noModel
              ? 'Pick a model in Settings › IDE › Commit messages, or choose one in the chat.'
              : gen.state.message
          }
          action={
            gen.state.noModel ? (
              <Button type="button" variant="pill" size="sm" onClick={() => setSettingsOpen(true)}>
                Open settings
              </Button>
            ) : (
              <Button type="button" variant="pill" size="sm" onClick={() => void generate()}>
                Retry
              </Button>
            )
          }
        />
      ) : null}

      <textarea
        className="shui-commit-message"
        aria-label="Commit message"
        placeholder={`Commit message (${navigator.platform.includes('Mac') ? '⌘' : 'Ctrl+'}Enter to commit)`}
        rows={5}
        spellCheck
        value={message}
        readOnly={writing}
        aria-busy={writing}
        onChange={(event) => setMessage(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
            event.preventDefault()
            void submit(false)
          }
        }}
      />

      <div className="shui-commit-status">
        {writing ? (
          <>
            <StatusDot tone="accent" pulse />
            <span className="shui-commit-writing">
              Writing from{' '}
              {gen.state.phase === 'writing' && gen.state.files === 1
                ? '1 file'
                : `${gen.state.phase === 'writing' ? gen.state.files : 0} files`}
            </span>
            <span className="spacer" />
            {gen.model ? <span className="mono">{modelLabel(gen.model)}</span> : null}
          </>
        ) : (
          <>
            <span className="num">{summary || 'Nothing selected'}</span>
            {replacedDraft !== null ? (
              <button
                type="button"
                className="shui-commit-link"
                onClick={() => {
                  setMessage(replacedDraft)
                  setReplacedDraft(null)
                }}
              >
                Restore draft
              </button>
            ) : null}
            <span className="spacer" />
            {subject.length > 0 ? (
              <span
                className="mono num"
                data-over={subject.length > SUBJECT_LIMIT || undefined}
                title="Subject line length"
              >
                {subject.length}/{SUBJECT_LIMIT}
              </span>
            ) : null}
          </>
        )}
      </div>

      {scm.note ? (
        <div className="shui-commit-note" data-failed={scm.note.failed || undefined} role="status">
          {scm.note.failed ? <CircleAlert aria-hidden /> : <Check aria-hidden />}
          <span>{scm.note.text}</span>
        </div>
      ) : null}

      <div className="shui-commit-actions">
        <Button type="submit" variant="primary" size="sm" disabled={!canCommit}>
          {amend ? 'Amend commit' : 'Commit'}
        </Button>
        <Button type="button" variant="pill" size="sm" disabled={!canCommit} onClick={() => void submit(true)}>
          {amend ? 'Amend and push' : 'Commit and push'}
        </Button>
        <span className="spacer" />
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <IconButton label="Commit options" className="shui-commit-options" data-custom={customOptions || undefined}>
              <Settings aria-hidden />
            </IconButton>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" side="top" className="shui-commit-menu">
            <DropdownMenuLabel>Commit options</DropdownMenuLabel>
            <DropdownMenuCheckboxItem
              checked={signOff}
              onCheckedChange={setSignOff}
              onSelect={(event) => event.preventDefault()}
            >
              Sign-off
            </DropdownMenuCheckboxItem>
            <DropdownMenuCheckboxItem
              checked={runHooks}
              onCheckedChange={setRunHooks}
              onSelect={(event) => event.preventDefault()}
            >
              Run Git hooks
            </DropdownMenuCheckboxItem>
            <DropdownMenuItem onSelect={() => setAuthorOpen(true)}>
              <span className="shui-commit-menu-text">Author…</span>
              <span className="shui-commit-menu-meta">{author.trim() || 'from git config'}</span>
            </DropdownMenuItem>
            {customOptions ? (
              <>
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  onSelect={() => {
                    setSignOff(false)
                    setRunHooks(true)
                    setAuthor('')
                  }}
                >
                  Reset to defaults
                </DropdownMenuItem>
              </>
            ) : null}
          </DropdownMenuContent>
        </DropdownMenu>
      </div>

      <TextDialog
        open={authorOpen}
        title="Commit author"
        description="Overrides the author of the next commits made here. Leave empty to use git config."
        label="Author"
        placeholder="Name <email@example.com>"
        initial={author}
        allowEmpty
        confirmLabel="Use author"
        onCancel={() => setAuthorOpen(false)}
        onConfirm={(value) => {
          setAuthor(value)
          setAuthorOpen(false)
        }}
      />
      <WorkerConfigurationDialog
        configurationId={settingsOpen ? CONFIGURATION_ID : null}
        onClose={() => {
          setSettingsOpen(false)
          gen.loadConfig()
        }}
      />
    </form>
  )
}
