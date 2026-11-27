import type { Host } from '@iii-dev/console-ui'
import { isLaunching, markAsked, markFolderNote, setLaunch } from './asked'
import { hubOf } from './hub'
import {
  activeSameKind,
  fixMessage,
  followUp,
  HANDOFF_POINTER,
  handoffMessage,
  type ImportChoice,
  importMessage,
  pullRequestMessage,
} from './model'
import type { Deploy, HandoffKind, HandoffResult, Status } from './types'

/**
 * Hands the work to the agent. The message is one plain sentence the person might have typed:
 * the worker registers the hand-off first (`iii-cloud::handoff`) and its marker leads the text;
 * every detail (problems, fixes, paths) is in `iii-cloud::status` and the skill. `text` replaces
 * the sentence: a request about the hand-off the agent may already be on, which the worker delivers to it.
 */
export async function handOff(
  host: Host,
  status: Status,
  kind: HandoffKind,
  deploy?: Deploy,
  choice?: ImportChoice,
  text?: string,
  /** `candidate`: an import for the folder Setup is adding, not the project in use. */
  more: {
    candidate?: boolean /** Call the worker even when a hand-off of this kind exists: it sends the open problems into that chat. */
    call?: boolean
  } = {},
): Promise<boolean> {
  const compose = host.chat?.compose
  if (!compose) return false
  // One launch at a time, and never a second session for a hand-off the agent is already on.
  if (isLaunching()) return false
  if (!text && !more.call && activeSameKind(status, kind)) {
    openAgentChat(host, status)
    return true
  }
  const sentence =
    text ??
    (kind === 'deploy_fix' && deploy
      ? fixMessage(deploy, status)
      : kind === 'import'
        ? importMessage(status, choice)
        : kind === 'pull_request'
          ? pullRequestMessage(status)
          : handoffMessage(status))
  const plain = [sentence, HANDOFF_POINTER].join('\n')
  setLaunch({ kind: 'launching' })
  let made: HandoffResult | undefined
  try {
    // The worker tries to deliver to the agent's own chat first: when it does, no new chat is opened.
    made = await host.iii.call<HandoffResult>('iii-cloud::handoff', {
      kind,
      text: plain,
      ...(status.check ? { check_id: status.check.id } : {}),
      ...(deploy ? { deployment_id: deploy.id } : {}),
      ...(more.candidate ? { candidate: true } : {}),
    })
  } catch {
    // an older worker has no hand-off: the message still goes
  }
  if (status.check) {
    markAsked(status.check.id)
  }
  // "Fix with your agent" is the person asking again.
  hubOf(host).markStopped(status.project?.environment?.id ?? '', false)
  // A hand-off the worker already had open delivers its own text: never start a second chat.
  if (made?.reused) {
    if (made.session_id) openChatFor(host, made.session_id)
    // `sent: false`: the chat is open but nothing was put in it: the person tells it.
    setLaunch(made.sent === false ? { kind: 'told' } : made.session_id ? { kind: 'sent' } : { kind: 'waiting' })
    return true
  }
  if (made?.delivered && made.session_id) {
    openChatFor(host, made.session_id)
    setLaunch({ kind: 'sent' })
    return true
  }
  // The agent is mid-run on something else: the worker turns this into a follow-up in its session.
  if (followUp(status, kind)) {
    setLaunch({ kind: 'waiting' })
    return true
  }
  const marked = [made?.marker, plain].filter(Boolean).join('\n')
  const retry = () => void handOff(host, status, kind, deploy, choice, text, more)
  try {
    const ok = await launchChat(host, status, marked, sentence)
    setLaunch(ok ? { kind: 'started' } : { kind: 'error', retry })
  } catch {
    setLaunch({ kind: 'error', retry })
  }
  return true
}

/** Starts the chat; false when the console could not take it (never silent). */
async function launchChat(host: Host, status: Status, text: string, title: string): Promise<boolean> {
  const compose = host.chat?.compose
  if (!compose) return false
  const folder = workFolder(status)
  const chat = host.chat as ChatSlots
  const refused = (r: unknown) => r === 'ambiguous' || r === 'unavailable'
  if (chat.startChat && chat.startChatWorkingDir === true && folder) {
    // (a) the console starts the chat in the worktree, on the cloud-ready profile
    await chat.startChat({ text, agent: 'cloud-ready', workingDir: folder })
    return true
  }
  if (chat.openDraft && chat.requestWorkingDirectoryChange && folder) {
    // (b) a draft chat, moved into the worktree by this click, then sent
    const id = chat.openDraft({ text: '', title }, { show: true })
    if (typeof id === 'string' && id) {
      const moved = chat.requestWorkingDirectoryChange({ sessionId: id, path: folder })
      const sent = await compose({ sessionId: id, text, submit: true })
      if (refused(sent)) return false
      if (!moved && status.check) markFolderNote(status.check.id)
      return true
    }
  }
  // (c) the chat in front of the person, wherever it is
  return !refused(await compose({ text, submit: true, beside: 'ext:iii-cloud' }))
}

type ChatSlots = NonNullable<Host['chat']> & {
  /** An ADE change: `startChat` accepts a folder. */
  startChatWorkingDir?: boolean
  startChat?: (request: { text: string; agent?: string; workingDir?: string }) => Promise<string>
}

/** Where the agent works: the worktree, or the checkout when there is none. */
export function workFolder(s: Status): string | null {
  const p = s.project
  if (!p) return null
  return p.workspace && p.workspace.mode !== 'checkout' ? p.workspace.path : p.path
}

export const canOpenChat = (host: Host) =>
  !!(host.chat?.openConversation || host.panels?.openScreen || host.chat?.selectConversation)

/**
 * Brings a session's chat forward without a second pane: `openConversation` reuses a pane that already shows it,
 * `openScreen` places or reuses a screen, and a host with neither selects the conversation.
 */
export function openChatFor(host: Host, sessionId: string): boolean {
  const chat = host.chat
  try {
    if (chat?.openConversation) chat.openConversation(sessionId)
    else if (host.panels?.openScreen) host.panels.openScreen({ screen: `chat:${sessionId}` })
    else if (chat?.selectConversation) chat.selectConversation(sessionId)
    else return false
    return true
  } catch {
    return false
  }
}

/** Brings the agent's chat forward (when the console can and the hand-off has a session). */
export function openAgentChat(host: Host, status: Status): boolean {
  const id = status.agent?.session_id
  return id ? openChatFor(host, id) : false
}
