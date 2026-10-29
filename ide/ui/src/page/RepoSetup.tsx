/* What the Commit tab shows for a folder without a repository, or a
   repository without a remote: three connected steps, one open at a time.
   Initialize Git (a first commit, a starter .gitignore), connect the GitHub
   CLI (installed, then signed in from a terminal or with a pasted token),
   and publish the folder as a GitHub repository. Steps 2 and 3 are
   optional; the folder is a working repository after step 1. */

import type { Host } from '@iii-dev/console-ui'
import { Button, Checkbox, IconButton, Input, uiClasses } from '@iii-dev/console-ui'
import { copyText, errorMessage } from '@iii-dev/console-ui/format'
import { ArrowUpRight, Check, CircleCheck, CloudUpload, Copy, FolderGit2, Globe, LoaderCircle, Lock, X } from 'lucide-react'
import { type FormEvent, type ReactNode, useCallback, useEffect, useId, useRef, useState } from 'react'
import { gitPush } from './git-actions'
import {
  GH_INSTALL_URL,
  GH_LOGIN_COMMAND,
  GH_TOKEN_URL,
  type GhStatus,
  ghCreateRepository,
  ghIdentity,
  ghInstallHint,
  ghSignInWithToken,
  ghStatus,
  gitInitialCommit,
  gitInitRepository,
  gitSetIdentity,
  hasGitignore,
  IdentityMissingError,
  NameTakenError,
  repoNameProblem,
  suggestRepoName,
  type Visibility,
} from './repo-setup'

type Step = 'git' | 'github' | 'create' | 'done'
type StepState = 'done' | 'active' | 'todo'

const ORDER: Step[] = ['git', 'github', 'create', 'done']

/** While the browser sign-in runs in a terminal, gh is asked this often. */
const SIGN_IN_POLL_MS = 3_000

interface RepoSetupProps {
  host: Host
  root: string
  /** `init`: no repository yet. `publish`: a repository with no remote. */
  start: 'init' | 'publish'
  /** `git init` succeeded: the page reads the folder's status again. */
  onInitialized: () => void
  /** Opens a terminal in the folder with `command` typed into it. */
  onRunInTerminal?: (command: string) => void
  /** Closes the setup: finished, or GitHub skipped. */
  onClose: () => void
}

function stateOf(step: Step, current: Step): StepState {
  const a = ORDER.indexOf(step)
  const b = ORDER.indexOf(current)
  return a < b ? 'done' : a === b ? 'active' : 'todo'
}

/** One step on the rail: a numbered mark that turns into a tick, the line
    to the next step (it fills once this one is done), a title, one line of
    detail, and the controls while it is the current step. */
function SetupStep({
  index,
  title,
  detail,
  summary,
  state,
  optional,
  last,
  children,
}: {
  index: number
  title: string
  detail: ReactNode
  summary: ReactNode
  state: StepState
  optional?: boolean
  last?: boolean
  children?: ReactNode
}) {
  return (
    <li className="shui-repo-step" data-state={state} aria-current={state === 'active' ? 'step' : undefined}>
      {last ? null : (
        <span className="shui-repo-step-rail" aria-hidden>
          <span className="shui-repo-step-rail-fill" />
        </span>
      )}
      <span className="shui-repo-step-mark" aria-hidden>
        <span className="shui-repo-step-num">{index}</span>
        <Check className="shui-repo-step-check" />
      </span>
      <div className="shui-repo-step-main">
        <div className="shui-repo-step-head">
          <span className="shui-repo-step-title">
            {state === 'done' ? <span className="shui-sr-only">Done: </span> : null}
            {title}
          </span>
          {optional && state !== 'done' ? <span className="shui-repo-step-tag">Optional</span> : null}
        </div>
        <div className="shui-repo-step-detail">{state === 'done' ? summary : detail}</div>
        {state === 'active' && children ? <div className="shui-repo-step-body">{children}</div> : null}
      </div>
    </li>
  )
}

/** A command to run, with a copy button. */
function CommandLine({ command }: { command: string }) {
  const [copied, setCopied] = useState(false)
  return (
    <div className="shui-repo-command">
      <span className="shui-repo-command-prompt" aria-hidden>
        $
      </span>
      <code>{command}</code>
      <IconButton
        type="button"
        label={copied ? 'Copied' : 'Copy command'}
        onClick={() => {
          void copyText(command).then((ok) => {
            if (!ok) return
            setCopied(true)
            setTimeout(() => setCopied(false), 1_500)
          })
        }}
      >
        <span className="shui-repo-swap" data-on={copied}>
          <Copy aria-hidden />
          <Check aria-hidden />
        </span>
      </IconButton>
    </div>
  )
}

function Note({ children, tone, id }: { children: ReactNode; tone?: 'alert'; id?: string }) {
  return (
    <p id={id} className={tone ? `shui-repo-note ${tone}` : 'shui-repo-note'} role={tone ? 'alert' : undefined}>
      {children}
    </p>
  )
}

function Waiting({ children }: { children: ReactNode }) {
  return (
    <p className="shui-repo-waiting" role="status">
      <LoaderCircle aria-hidden className={uiClasses.spin} />
      {children}
    </p>
  )
}

function ExternalLink({ href, children, mono }: { href: string; children: ReactNode; mono?: boolean }) {
  return (
    <a className={mono ? 'shui-repo-link mono' : 'shui-repo-link'} href={href} target="_blank" rel="noreferrer">
      <span>{children}</span>
      <ArrowUpRight aria-hidden />
    </a>
  )
}

const VISIBILITY: { value: Visibility; title: string; detail: string; Icon: typeof Lock }[] = [
  { value: 'private', title: 'Private', detail: 'Only you and people you invite can see it', Icon: Lock },
  { value: 'public', title: 'Public', detail: 'Anyone on the internet can see it', Icon: Globe },
]

export function RepoSetup({ host, root, start, onInitialized, onRunInTerminal, onClose }: RepoSetupProps) {
  const [step, setStep] = useState<Step>(start === 'init' ? 'git' : 'github')
  const ids = useId()

  // ── step 1: git init ──────────────────────────────────────────────
  const [needsGitignore, setNeedsGitignore] = useState(false)
  const [addGitignore, setAddGitignore] = useState(true)
  const [initBusy, setInitBusy] = useState(false)
  const [initError, setInitError] = useState<string | null>(null)
  const [wroteGitignore, setWroteGitignore] = useState(false)
  const [committed, setCommitted] = useState(false)
  const [identity, setIdentity] = useState<{ name: string; email: string } | null>(null)
  const [identityFromGitHub, setIdentityFromGitHub] = useState(false)

  useEffect(() => {
    if (start !== 'init') return
    let live = true
    void hasGitignore(host, root)
      .then((has) => live && setNeedsGitignore(!has))
      .catch(() => undefined)
    return () => {
      live = false
    }
  }, [host, root, start])

  const initialize = async () => {
    setInitBusy(true)
    setInitError(null)
    try {
      const result = await gitInitRepository(host, root, { gitignore: needsGitignore && addGitignore, commit: false })
      setWroteGitignore(result.wroteGitignore)
      onInitialized()
      setCommitted(await gitInitialCommit(host, root))
      onInitialized()
      setStep('github')
    } catch (error) {
      if (error instanceof IdentityMissingError) {
        const suggested = await ghIdentity(host, root)
        setIdentityFromGitHub(suggested !== null)
        setIdentity(suggested ?? { name: '', email: '' })
      } else {
        setInitError(errorMessage(error))
      }
    } finally {
      setInitBusy(false)
    }
  }

  const saveIdentity = async (event: FormEvent) => {
    event.preventDefault()
    if (!identity) return
    setInitBusy(true)
    setInitError(null)
    try {
      await gitSetIdentity(host, root, identity.name, identity.email)
      setCommitted(await gitInitialCommit(host, root))
      setIdentity(null)
      onInitialized()
      setStep('github')
    } catch (error) {
      setInitError(errorMessage(error))
    } finally {
      setInitBusy(false)
    }
  }

  // ── step 2: the GitHub CLI ────────────────────────────────────────
  const [gh, setGh] = useState<GhStatus | null>(null)
  const [checking, setChecking] = useState(false)
  const [signingIn, setSigningIn] = useState(false)
  const [tokenMode, setTokenMode] = useState(false)
  const [token, setToken] = useState('')
  const [tokenBusy, setTokenBusy] = useState(false)
  const [tokenError, setTokenError] = useState<string | null>(null)
  const checkSeq = useRef(0)

  const check = useCallback(async () => {
    const seq = ++checkSeq.current
    setChecking(true)
    const status = await ghStatus(host, root)
    if (seq !== checkSeq.current) return
    setChecking(false)
    setGh(status)
  }, [host, root])

  // Read gh's state ahead of step 2, so it opens already knowing.
  useEffect(() => {
    void check()
  }, [check])

  // Signed in: step 2 is done by itself.
  useEffect(() => {
    if (step === 'github' && gh?.kind === 'signed-in') {
      setSigningIn(false)
      setStep('create')
    }
  }, [step, gh])

  // The sign-in finishes in the terminal and the browser: ask gh again on a
  // timer and whenever the page comes back into view.
  useEffect(() => {
    if (step !== 'github' || !signingIn) return
    const timer = setInterval(() => void check(), SIGN_IN_POLL_MS)
    const onFocus = () => void check()
    window.addEventListener('focus', onFocus)
    return () => {
      clearInterval(timer)
      window.removeEventListener('focus', onFocus)
    }
  }, [step, signingIn, check])

  const signInWithBrowser = () => {
    onRunInTerminal?.(GH_LOGIN_COMMAND)
    setTokenMode(false)
    setSigningIn(true)
  }

  const connectToken = async (event: FormEvent) => {
    event.preventDefault()
    setTokenBusy(true)
    setTokenError(null)
    try {
      const login = await ghSignInWithToken(host, root, token)
      setToken('')
      setGh({ kind: 'signed-in', login })
    } catch (error) {
      setTokenError(errorMessage(error))
    } finally {
      setTokenBusy(false)
    }
  }

  // ── step 3: gh repo create ────────────────────────────────────────
  const [name, setName] = useState(() => suggestRepoName(root))
  const [description, setDescription] = useState('')
  const [visibility, setVisibility] = useState<Visibility>('private')
  const [createBusy, setCreateBusy] = useState(false)
  const [createError, setCreateError] = useState<string | null>(null)
  const [takenName, setTakenName] = useState<string | null>(null)
  const [published, setPublished] = useState<{ url: string | null; pushError: string | null } | null>(null)
  const [pushBusy, setPushBusy] = useState(false)
  const [nameTouched, setNameTouched] = useState(false)
  const nameProblem =
    repoNameProblem(name) ??
    (takenName !== null && takenName === name.trim()
      ? `You already have a repository named ${takenName}. Choose another name.`
      : null)
  const login = gh?.kind === 'signed-in' ? gh.login : null

  const create = async (event: FormEvent) => {
    event.preventDefault()
    setNameTouched(true)
    if (nameProblem) return
    setCreateBusy(true)
    setCreateError(null)
    try {
      const result = await ghCreateRepository(host, root, { name, description, visibility })
      setPublished(result)
      onInitialized()
      setStep('done')
    } catch (error) {
      if (error instanceof NameTakenError) setTakenName(name.trim())
      else setCreateError(errorMessage(error))
    } finally {
      setCreateBusy(false)
    }
  }

  const pushAgain = async () => {
    setPushBusy(true)
    try {
      await gitPush(host, root)
      setPublished((current) => (current ? { ...current, pushError: null } : current))
      onInitialized()
    } catch (error) {
      setPublished((current) => (current ? { ...current, pushError: errorMessage(error) } : current))
    } finally {
      setPushBusy(false)
    }
  }

  const install = ghInstallHint(typeof navigator === 'undefined' ? '' : navigator.platform)
  const repoPath = published?.url?.replace(/^https:\/\//, '') ?? null
  const gitSummary =
    start === 'init'
      ? ['On main', committed ? 'First commit saved' : 'No files to commit yet', wroteGitignore ? '.gitignore added' : null]
          .filter(Boolean)
          .join(' · ')
      : 'Already set up'

  return (
    <section className="shui-repo-setup" aria-labelledby={`${ids}-title`}>
      <header className="shui-repo-setup-head">
        <span className="shui-repo-setup-mark" aria-hidden>
          {start === 'init' ? <FolderGit2 /> : <CloudUpload />}
        </span>
        {start === 'publish' && step !== 'done' ? (
          <IconButton className="shui-repo-setup-close" label="Close" onClick={onClose}>
            <X aria-hidden />
          </IconButton>
        ) : null}
        <h2 id={`${ids}-title`} className="shui-repo-setup-title">
          {start === 'init' ? 'Start tracking this folder' : 'Back up on GitHub'}
        </h2>
        <p className="shui-repo-setup-lede">
          {start === 'init'
            ? 'Keep a history of every change, then back it up on GitHub.'
            : 'This repository only lives on this machine. Publish it to keep a copy safe and share it.'}
        </p>
      </header>

      <ol className="shui-repo-steps">
        <SetupStep
          index={1}
          title="Track changes with Git"
          detail="Starts a history on the main branch, with your current files as the first commit."
          summary={gitSummary}
          state={stateOf('git', step)}
        >
          {identity ? (
            <form className="shui-repo-form" onSubmit={(event) => void saveIdentity(event)}>
              <div className="shui-repo-callout">
                <p className="shui-repo-callout-title">Add your name to commits</p>
                <Note>
                  {identityFromGitHub
                    ? 'Filled in from your GitHub account, with its private noreply email. Saved to your global Git config.'
                    : 'Every commit records who made it. Saved to your global Git config.'}
                </Note>
              </div>
              <div className="shui-repo-field">
                <label className="shui-repo-label" htmlFor={`${ids}-git-name`}>
                  Name
                </label>
                <Input
                  id={`${ids}-git-name`}
                  value={identity.name}
                  onChange={(value) => setIdentity((current) => current && { ...current, name: value })}
                  placeholder="Ada Lovelace"
                  autoComplete="name"
                  autoFocus
                />
              </div>
              <div className="shui-repo-field">
                <label className="shui-repo-label" htmlFor={`${ids}-git-email`}>
                  Email
                </label>
                <Input
                  id={`${ids}-git-email`}
                  type="email"
                  value={identity.email}
                  onChange={(value) => setIdentity((current) => current && { ...current, email: value })}
                  placeholder="ada@example.com"
                  autoComplete="email"
                />
              </div>
              {initError ? <Note tone="alert">{initError}</Note> : null}
              <Button
                type="submit"
                variant="primary"
                size="md"
                className="shui-repo-cta"
                disabled={initBusy || !identity.name.trim() || !/.+@.+/.test(identity.email)}
              >
                {initBusy ? 'Saving…' : 'Save and commit'}
              </Button>
            </form>
          ) : (
            <>
              {needsGitignore ? (
                <Checkbox
                  className="shui-repo-check"
                  label="Ignore dependencies, build output and .env files"
                  checked={addGitignore}
                  onChange={(event) => setAddGitignore(event.target.checked)}
                />
              ) : null}
              {initError ? <Note tone="alert">{initError}</Note> : null}
              <Button
                type="button"
                variant="primary"
                size="md"
                className="shui-repo-cta"
                disabled={initBusy}
                onClick={() => void initialize()}
              >
                {initBusy ? 'Initializing…' : 'Initialize repository'}
              </Button>
            </>
          )}
        </SetupStep>

        <SetupStep
          index={2}
          title="Connect GitHub"
          detail="Sign in once and every project on this machine can use it."
          summary={login ? `Signed in as @${login}` : 'Connected'}
          state={stateOf('github', step)}
          optional
        >
          {gh === null ? (
            <Waiting>Looking for the GitHub CLI…</Waiting>
          ) : gh.kind === 'missing' ? (
            <>
              <div className="shui-repo-callout">
                <p className="shui-repo-callout-title">Install the GitHub CLI</p>
                <Note>The IDE uses it to sign in and create repositories for you.</Note>
              </div>
              {install.command ? <CommandLine command={install.command} /> : null}
              {install.command && onRunInTerminal ? (
                <Button
                  type="button"
                  variant="primary"
                  size="md"
                  className="shui-repo-cta"
                  onClick={() => onRunInTerminal(install.command ?? '')}
                >
                  Install with {install.label}
                </Button>
              ) : null}
              <div className="shui-repo-row">
                <Button type="button" variant="ghost" size="sm" disabled={checking} onClick={() => void check()}>
                  {checking ? 'Checking…' : 'Check again'}
                </Button>
                <ExternalLink href={GH_INSTALL_URL}>Other ways to install</ExternalLink>
              </div>
            </>
          ) : gh.kind === 'error' ? (
            <>
              <Note tone="alert">{gh.message}</Note>
              <Button
                type="button"
                variant="secondary"
                size="md"
                className="shui-repo-cta"
                disabled={checking}
                onClick={() => void check()}
              >
                {checking ? 'Checking…' : 'Try again'}
              </Button>
            </>
          ) : gh.kind === 'signed-out' ? (
            tokenMode || !onRunInTerminal ? (
              <form className="shui-repo-form" onSubmit={(event) => void connectToken(event)}>
                <ol className="shui-repo-substeps">
                  <li>
                    <ExternalLink href={GH_TOKEN_URL}>Create a token on GitHub</ExternalLink>
                    <span className="shui-repo-substep-detail">
                      The <code>repo</code> and <code>read:org</code> scopes are already selected.
                    </span>
                  </li>
                  <li>Copy the token and paste it below.</li>
                </ol>
                <div className="shui-repo-field">
                  <label className="shui-repo-label" htmlFor={`${ids}-token`}>
                    Personal access token
                  </label>
                  <Input
                    id={`${ids}-token`}
                    type="password"
                    value={token}
                    onChange={setToken}
                    placeholder="ghp_xxxxxxxxxxxx"
                    autoComplete="off"
                    spellCheck={false}
                    aria-invalid={tokenError !== null}
                    aria-describedby={tokenError ? `${ids}-token-error` : undefined}
                  />
                  {tokenError ? (
                    <Note tone="alert" id={`${ids}-token-error`}>
                      {tokenError}
                    </Note>
                  ) : null}
                </div>
                <Button
                  type="submit"
                  variant="primary"
                  size="md"
                  className="shui-repo-cta"
                  disabled={tokenBusy || token.trim() === ''}
                >
                  {tokenBusy ? 'Connecting…' : 'Connect'}
                </Button>
                {onRunInTerminal ? (
                  <button type="button" className="shui-repo-text-button" onClick={signInWithBrowser}>
                    Sign in with the browser instead
                  </button>
                ) : null}
              </form>
            ) : signingIn ? (
              <>
                <ol className="shui-repo-substeps">
                  <li>In the terminal below, press Enter to open GitHub.</li>
                  <li>Paste the one-time code from the terminal.</li>
                  <li>Select Authorize.</li>
                </ol>
                <Waiting>Waiting for you to finish in the browser…</Waiting>
                <div className="shui-repo-row">
                  <Button type="button" variant="ghost" size="sm" disabled={checking} onClick={() => void check()}>
                    Check again
                  </Button>
                  <Button type="button" variant="ghost" size="sm" onClick={signInWithBrowser}>
                    Open a new terminal
                  </Button>
                </div>
              </>
            ) : (
              <>
                <Button type="button" variant="primary" size="md" className="shui-repo-cta" onClick={signInWithBrowser}>
                  Sign in with GitHub
                </Button>
                <button type="button" className="shui-repo-text-button" onClick={() => setTokenMode(true)}>
                  Use a personal access token instead
                </button>
              </>
            )
          ) : null}
        </SetupStep>

        <SetupStep
          index={3}
          title="Publish to GitHub"
          detail="Creates the repository on GitHub and pushes your commits to it."
          summary={
            published?.url && repoPath ? (
              <ExternalLink href={published.url} mono>
                {repoPath}
              </ExternalLink>
            ) : (
              'Published'
            )
          }
          state={stateOf('create', step)}
          last
        >
          <form className="shui-repo-form" onSubmit={(event) => void create(event)} noValidate>
            <div className="shui-repo-field">
              <label className="shui-repo-label" htmlFor={`${ids}-name`}>
                Repository name
              </label>
              <Input
                id={`${ids}-name`}
                value={name}
                onChange={(value) => {
                  setName(value)
                  setNameTouched(true)
                }}
                spellCheck={false}
                autoComplete="off"
                aria-invalid={nameTouched && nameProblem !== null}
                aria-describedby={`${ids}-name-hint`}
              />
              {nameTouched && nameProblem ? (
                <Note tone="alert" id={`${ids}-name-hint`}>
                  {nameProblem}
                </Note>
              ) : (
                <p id={`${ids}-name-hint`} className="shui-repo-path">
                  {login ? (
                    <>
                      github.com/{login}/<strong>{name.trim() || 'name'}</strong>
                    </>
                  ) : null}
                </p>
              )}
            </div>
            <fieldset className="shui-repo-field shui-repo-fieldset">
              <legend className="shui-repo-label">Visibility</legend>
              <div className="shui-repo-choices">
                {VISIBILITY.map(({ value, title, detail, Icon }) => (
                  <label key={value} className="shui-repo-choice">
                    <input
                      type="radio"
                      name={`${ids}-visibility`}
                      value={value}
                      checked={visibility === value}
                      onChange={() => setVisibility(value)}
                      className="shui-sr-only"
                    />
                    <Icon aria-hidden className="shui-repo-choice-icon" />
                    <span className="shui-repo-choice-text">
                      <span className="shui-repo-choice-title">{title}</span>
                      <span className="shui-repo-choice-detail">{detail}</span>
                    </span>
                    <span className="shui-repo-choice-radio" aria-hidden />
                  </label>
                ))}
              </div>
            </fieldset>
            <div className="shui-repo-field">
              <label className="shui-repo-label" htmlFor={`${ids}-description`}>
                Description <span className="shui-repo-optional">Optional</span>
              </label>
              <Input
                id={`${ids}-description`}
                value={description}
                onChange={setDescription}
                autoComplete="off"
              />
            </div>
            {createError ? <Note tone="alert">{createError}</Note> : null}
            <Button type="submit" variant="primary" size="md" className="shui-repo-cta" disabled={createBusy}>
              {createBusy ? 'Publishing…' : 'Publish repository'}
            </Button>
          </form>
        </SetupStep>
      </ol>

      {step === 'done' && published ? (
        <div className="shui-repo-done" role="status">
          <span className="shui-repo-done-mark" aria-hidden>
            <CircleCheck />
          </span>
          <p className="shui-repo-done-title">Your repository is on GitHub</p>
          {published.pushError ? (
            <Note tone="alert">The first push didn't go through: {published.pushError}</Note>
          ) : (
            <Note>Commit here, then push to keep it up to date.</Note>
          )}
          <div className="shui-repo-done-actions">
            {published.pushError ? (
              <Button type="button" variant="primary" size="md" disabled={pushBusy} onClick={() => void pushAgain()}>
                {pushBusy ? 'Pushing…' : 'Try pushing again'}
              </Button>
            ) : published.url ? (
              <Button
                type="button"
                variant="secondary"
                size="md"
                onClick={() => window.open(published.url ?? '', '_blank', 'noopener')}
              >
                Open on GitHub
              </Button>
            ) : null}
            <Button type="button" variant={published.pushError ? 'secondary' : 'primary'} size="md" onClick={onClose}>
              Done
            </Button>
          </div>
        </div>
      ) : null}

      {start === 'init' && (step === 'github' || step === 'create') ? (
        <footer className="shui-repo-setup-foot">
          <button type="button" className="shui-repo-text-button" onClick={onClose}>
            Keep it local for now
          </button>
        </footer>
      ) : null}
    </section>
  )
}
