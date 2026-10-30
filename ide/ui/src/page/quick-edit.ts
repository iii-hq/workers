/* Quick Edit's model call: the editor's ⌘K prompt hands over the
   instruction, the picked code and the file around it; this asks the
   router for the replacement and cleans the answer up. The model is the
   one the chat composer has picked (what the person already talks to),
   else the first configured provider's default, like the voice clean-up. */

import type { CodeEditorQuickEditRequest, Host } from '@iii-dev/console-ui'

/** Lines of the file shown around the picked code, each side. */
export const CONTEXT_LINES = 120
/** A reasoning model rewriting a big selection can take a while. */
const COMPLETE_TIMEOUT_MS = 120_000
const PROVIDER_LIST_TIMEOUT_MS = 5_000

export const QUICK_EDIT_SYSTEM_PROMPT = `You edit code inside an editor. The user shows a file, marks the code to change, and says what to change.
Reply with the replacement for the marked code only: the full new text that goes between the markers, nothing before or after it.
No explanations, no Markdown fences, no markers.
Keep the surrounding indentation, the file's style and everything the instruction does not ask you to change.
If the marked code is the whole file, reply with the whole new file.`

const START = '<<<EDIT_START>>>'
const END = '<<<EDIT_END>>>'

/** The user message: the instruction, then the file with the target marked. */
export function buildQuickEditPrompt(request: Pick<CodeEditorQuickEditRequest, 'instruction' | 'scope' | 'selection' | 'value' | 'language'>, path: string): string {
  const { instruction, scope, selection, value, language } = request
  const lines = value.split('\n')
  let file: string
  if (scope === 'file') {
    file = `${START}\n${value}\n${END}`
  } else {
    const startOffset = offsetAt(lines, selection.startLine, selection.startColumn)
    const endOffset = offsetAt(lines, selection.endLine, selection.endColumn)
    const firstShown = Math.max(1, selection.startLine - CONTEXT_LINES)
    const lastShown = Math.min(lines.length, selection.endLine + CONTEXT_LINES)
    const shownStart = offsetAt(lines, firstShown, 1)
    const shownEnd = offsetAt(lines, lastShown, (lines[lastShown - 1]?.length ?? 0) + 1)
    file =
      (firstShown > 1 ? `… (${firstShown - 1} lines above)\n` : '') +
      value.slice(shownStart, startOffset) +
      START +
      value.slice(startOffset, endOffset) +
      END +
      value.slice(endOffset, shownEnd) +
      (lastShown < lines.length ? `\n… (${lines.length - lastShown} lines below)` : '')
  }
  return [
    `Instruction: ${instruction}`,
    '',
    `File: ${path} (${language})`,
    `The code to replace is between ${START} and ${END}.`,
    '',
    file,
  ].join('\n')
}

/** Character offset of 1-based `line`:`column` in the joined text. */
function offsetAt(lines: readonly string[], line: number, column: number): number {
  let offset = 0
  for (let index = 0; index < line - 1 && index < lines.length; index += 1) offset += lines[index].length + 1
  return offset + column - 1
}

/** Models wrap code in a fence (or echo the markers) now and then despite
    the prompt: unwrap one whole-answer fence and drop stray markers, then
    match the original's trailing newline so the edit does not add or eat a
    line break. */
export function cleanQuickEditAnswer(answer: string, original: string): string {
  let text = answer.replaceAll(START, '').replaceAll(END, '')
  const fenced = /^\s*```[^\n`]*\n([\s\S]*?)\n?```\s*$/.exec(text)
  if (fenced && !fenced[1].includes('```')) text = fenced[1]
  const wantsNewline = original.endsWith('\n')
  if (wantsNewline && !text.endsWith('\n')) text += '\n'
  if (!wantsNewline) text = text.replace(/\n+$/, '')
  return text
}

/** `provider::model` → its two halves; a bare id has no provider. */
export function splitModel(id: string): { provider: string | null; model: string } {
  const at = id.indexOf('::')
  return at === -1 ? { provider: null, model: id } : { provider: id.slice(0, at), model: id.slice(at + 2) }
}

interface ProviderList {
  providers?: { id?: string; configured?: boolean; available?: boolean; default_model?: string }[]
}

/** The chat composer's model, else the first configured provider's default. */
export async function resolveQuickEditModel(host: Host, conversationId: string | null | undefined): Promise<string> {
  const chosen = host.chat?.composerModel?.(conversationId ?? null)
  if (chosen) return chosen
  const list = await host.iii.trigger<ProviderList>('router::provider::list', {}, { timeoutMs: PROVIDER_LIST_TIMEOUT_MS })
  const provider = list.providers?.find((p) => p.configured && p.available && p.id && p.default_model)
  if (!provider?.id || !provider.default_model) {
    throw new Error('No model is configured. Add a provider key in Settings, or pick a model in chat.')
  }
  return `${provider.id}::${provider.default_model}`
}

interface CompleteReply {
  message?: {
    stop_reason?: string
    error_message?: string
    content?: { type?: string; text?: string }[] | string
  }
}

/** The text blocks of a `router::complete` answer, joined. */
export function replyText(reply: CompleteReply): string {
  const content = reply.message?.content
  if (typeof content === 'string') return content
  return (content ?? []).filter((block) => block.type === 'text').map((block) => block.text ?? '').join('')
}

/** Room for the rewrite: generous, bounded so a runaway reply cannot stall. */
export function maxOutputTokens(original: string): number {
  return Math.min(16_000, Math.max(1024, Math.ceil(original.length / 2) + 512))
}

function abortError(): Error {
  return Object.assign(new Error('stopped'), { name: 'AbortError' })
}

/** Ask the model for the replacement. Rejects with a readable message. */
export async function runQuickEdit(
  host: Host,
  request: CodeEditorQuickEditRequest,
  path: string,
  model: string,
): Promise<string> {
  if (request.signal.aborted) throw abortError()
  const original = request.scope === 'file' ? request.value : request.selection.text
  const { provider, model: modelId } = splitModel(model)
  const call = host.iii.trigger<CompleteReply>(
    'router::complete',
    {
      model: modelId,
      ...(provider ? { provider } : {}),
      system_prompt: QUICK_EDIT_SYSTEM_PROMPT,
      messages: [
        { role: 'user', content: [{ type: 'text', text: buildQuickEditPrompt(request, path) }], timestamp: Date.now() },
      ],
      max_output_tokens: maxOutputTokens(original),
      ...(request.thinkingLevel && request.thinkingLevel !== 'default' ? { thinking_level: request.thinkingLevel } : {}),
    },
    { timeoutMs: COMPLETE_TIMEOUT_MS },
  )
  // The router cannot be cancelled from here; stopping just stops waiting.
  const stopped = new Promise<never>((_, reject) => {
    request.signal.addEventListener('abort', () => reject(abortError()), { once: true })
  })
  const reply = await Promise.race([call, stopped])
  if (reply.message?.stop_reason === 'error') {
    throw new Error(reply.message.error_message || 'The model ended its turn with an error.')
  }
  const text = replyText(reply)
  if (text.trim() === '' && original.trim() !== '') throw new Error('The model returned no code.')
  return cleanQuickEditAnswer(text, original)
}
