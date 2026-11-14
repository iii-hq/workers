import type { FunctionCallMessage } from '@iii-dev/console-ui'
import { describe, expect, it } from 'vitest'
import {
  askCardState,
  type AskQuestionView,
  type AskView,
  DISMISS_TEXT,
  formatAnswer,
  movedOn,
  parseAsk,
  sendLanded,
  settlementsOf,
  SKIPPED_ANSWER,
  type TurnPosition,
  turnPositionOf,
} from './ask'

/**
 * The row the console hands a renderer: the host contract plus the extra
 * fields ade/web's entry-mapper sets on it (`functionCallId`,
 * `sessionId`, `resultEntryId`, `unloaded`).
 */
type ConsoleRow = FunctionCallMessage & {
  functionCallId?: string
  sessionId?: string
  resultEntryId?: string
  unloaded?: boolean
}

/** The model's arguments (the `payload` inside agent_call). */
const REQUEST = {
  questions: [
    {
      header: 'Approach',
      question: 'When the agent asks, does the turn pause or end?',
      multi_select: false,
      options: [
        { label: 'Pause the turn', description: 'Like AskUserQuestion' },
        { label: 'End the turn', description: 'The answer becomes the next message' },
      ],
    },
    {
      header: 'Channels',
      question: 'Where should the card show up?',
      multi_select: true,
      options: [{ label: 'Console' }, { label: 'Slack' }, { label: 'Telegram' }],
    },
  ],
}

/** `ask::awaiting_result` details: `multi_select` always present,
 *  `description` omitted when absent. */
const DETAILS = {
  status: 'awaiting_answer',
  question_id: 'call_ask_1',
  session_id: 'sess_1',
  turn_id: 'turn_7',
  questions: REQUEST.questions,
}

const AWAITING_TEXT =
  'The questions are now shown to the user as a card of options. ' +
  'Their answer arrives as their next message. ' +
  'Do not repeat the questions in text. End your turn now.'

/** entry-mapper `functionResultOutput`, non-error branch: `{ content, details }`. */
function okOutput(details: unknown): unknown {
  return { content: [{ type: 'text', text: AWAITING_TEXT }], details }
}

/** A settled row for an agent_call-wrapped call: `unwrapFunctionCall`
 *  lifts `function`/`payload`/`description`; the result pairing sets
 *  `output`, `running: false`, `pendingApproval: false`, `resultEntryId`. */
function wrappedRow(output: unknown): ConsoleRow {
  return {
    id: 'e_asst_3:1',
    role: 'function-call',
    functionId: 'harness::ask',
    description: 'Ask which approach to take',
    input: REQUEST,
    output,
    running: false,
    pendingApproval: false,
    functionCallId: 'call_ask_1',
    sessionId: 'sess_1',
    resultEntryId: 'e_res_4',
    createdAt: 1_790_000_000_000,
  }
}

const VIEW: AskView = {
  questionId: 'call_ask_1',
  sessionId: 'sess_1',
  turnId: 'turn_7',
  questions: [
    {
      header: 'Approach',
      question: 'When the agent asks, does the turn pause or end?',
      multiSelect: false,
      options: [
        { label: 'Pause the turn', description: 'Like AskUserQuestion' },
        { label: 'End the turn', description: 'The answer becomes the next message' },
      ],
    },
    {
      header: 'Channels',
      question: 'Where should the card show up?',
      multiSelect: true,
      options: [{ label: 'Console' }, { label: 'Slack' }, { label: 'Telegram' }],
    },
  ],
}

describe('parseAsk', () => {
  it('builds the view from an accepted ask wrapped in agent_call', () => {
    expect(parseAsk(wrappedRow(okOutput(DETAILS)))).toStrictEqual(VIEW)
  })

  it('builds the view from a native call (input is the raw arguments)', () => {
    const row: ConsoleRow = {
      id: 'e_asst_3:0',
      role: 'function-call',
      functionId: 'harness::ask',
      input: REQUEST,
      output: okOutput(DETAILS),
      running: false,
      pendingApproval: false,
      functionCallId: 'call_ask_1',
      createdAt: 1_790_000_000_000,
    }
    expect(parseAsk(row)).toStrictEqual(VIEW)
  })

  it('reads the questions from the result details, not the request', () => {
    // A paged read can leave the call's arguments out (`unloaded`, input
    // undefined) while its result is already paired in.
    const row: ConsoleRow = { ...wrappedRow(okOutput(DETAILS)), input: undefined, unloaded: true }
    expect(parseAsk(row)).toStrictEqual(VIEW)
  })

  it('accepts the relayed envelope that also carries `terminate`', () => {
    const output = { content: [{ type: 'text', text: AWAITING_TEXT }], details: DETAILS, terminate: false }
    expect(parseAsk(wrappedRow(output))).toStrictEqual(VIEW)
  })

  it('treats a null description as absent and a missing multi_select as false', () => {
    const details = {
      ...DETAILS,
      questions: [
        {
          header: 'Pick',
          question: 'Which one?',
          options: [{ label: 'A', description: null }, { label: 'B' }],
        },
      ],
    }
    expect(parseAsk(wrappedRow(okOutput(details)))?.questions).toStrictEqual([
      { header: 'Pick', question: 'Which one?', multiSelect: false, options: [{ label: 'A' }, { label: 'B' }] },
    ])
  })

  it('returns null for an error output (a refused ask)', () => {
    const reason = 'only one harness::ask per step; put all your questions (up to 4) in one call'
    // entry-mapper `functionResultOutput`, is_error branch.
    const output = {
      error: {
        kind: 'function_error',
        message: reason,
        details: null,
        content: [{ type: 'text', text: reason }],
      },
    }
    expect(parseAsk(wrappedRow(output))).toBeNull()
  })

  it('returns null while the call is still running with no output', () => {
    const row: ConsoleRow = { ...wrappedRow(undefined), running: true }
    delete row.output
    delete row.resultEntryId
    expect(parseAsk(row)).toBeNull()
  })

  it('returns null while the arguments are still streaming', () => {
    const row: ConsoleRow = {
      ...wrappedRow(undefined),
      input: { questions: [], _streaming: '{"questions":[{"header":"Appr' },
      running: true,
    }
    delete row.output
    expect(parseAsk(row)).toBeNull()
  })

  const q0 = REQUEST.questions[0]
  const malformed: Array<[string, unknown]> = [
    ['details is null', null],
    ['status is missing', { ...DETAILS, status: undefined }],
    ['status is unknown', { ...DETAILS, status: 'answered' }],
    ['question_id is missing', { ...DETAILS, question_id: undefined }],
    ['session_id is not a string', { ...DETAILS, session_id: 42 }],
    ['turn_id is empty', { ...DETAILS, turn_id: '' }],
    ['questions is not an array', { ...DETAILS, questions: 'nope' }],
    ['questions is empty', { ...DETAILS, questions: [] }],
    ['a question is not an object', { ...DETAILS, questions: ['Approach?'] }],
    ['header is missing', { ...DETAILS, questions: [{ ...q0, header: undefined }] }],
    ['question is empty', { ...DETAILS, questions: [{ ...q0, question: '' }] }],
    ['multi_select is not a boolean', { ...DETAILS, questions: [{ ...q0, multi_select: 'yes' }] }],
    ['options is missing', { ...DETAILS, questions: [{ ...q0, options: undefined }] }],
    ['options is empty', { ...DETAILS, questions: [{ ...q0, options: [] }] }],
    ['an option has no label', { ...DETAILS, questions: [{ ...q0, options: [{ description: 'x' }, { label: 'B' }] }] }],
    ['a description is a number', { ...DETAILS, questions: [{ ...q0, options: [{ label: 'A', description: 1 }, { label: 'B' }] }] }],
  ]

  it.each(malformed)('returns null when %s', (_case, details) => {
    expect(parseAsk(wrappedRow(okOutput(details)))).toBeNull()
  })
})

const APPROACH: AskQuestionView = {
  header: 'Approach',
  question: 'When the agent asks, does the turn pause or end?',
  multiSelect: false,
  options: [
    { label: 'Pause the turn', description: 'Like AskUserQuestion' },
    { label: 'End the turn', description: 'The answer becomes the next message' },
  ],
}

const CHANNELS: AskQuestionView = {
  header: 'Channels',
  question: 'Where should the card show up?',
  multiSelect: true,
  options: [{ label: 'Console' }, { label: 'Slack' }, { label: 'Telegram' }],
}

function viewOf(...questions: AskQuestionView[]): AskView {
  return { questionId: 'call_ask_1', sessionId: 'sess_1', turnId: 'turn_7', questions }
}

describe('formatAnswer', () => {
  it('answers a single-select with the picked label', () => {
    expect(formatAnswer(viewOf(APPROACH), [{ picked: ['End the turn'] }])).toBe(
      'Approach: End the turn',
    )
  })

  it('joins multi-select picks in option order, not click order', () => {
    expect(formatAnswer(viewOf(CHANNELS), [{ picked: ['Telegram', 'Console'] }])).toBe(
      'Channels: Console, Telegram',
    )
  })

  it('answers a single-select with the trimmed Other text', () => {
    expect(formatAnswer(viewOf(APPROACH), [{ picked: [], other: '  Ask me later  ' }])).toBe(
      'Approach: Ask me later',
    )
  })

  it('on a single-select, typed Other text replaces the pick', () => {
    expect(
      formatAnswer(viewOf(APPROACH), [{ picked: ['Pause the turn'], other: 'Neither' }]),
    ).toBe('Approach: Neither')
  })

  it('appends the Other text after the picks on a multi-select', () => {
    expect(
      formatAnswer(viewOf(CHANNELS), [{ picked: ['Slack', 'Console'], other: ' Email ' }]),
    ).toBe('Channels: Console, Slack, Email')
  })

  it('gives one line per question, in question order', () => {
    expect(
      formatAnswer(viewOf(APPROACH, CHANNELS), [
        { picked: ['End the turn'] },
        { picked: ['Slack'] },
      ]),
    ).toBe('Approach: End the turn\nChannels: Slack')
  })

  it('returns null when any question is unanswered', () => {
    expect(
      formatAnswer(viewOf(APPROACH, CHANNELS), [{ picked: ['End the turn'] }, { picked: [] }]),
    ).toBeNull()
  })

  it('returns null when a question has no selection entry at all', () => {
    expect(formatAnswer(viewOf(APPROACH, CHANNELS), [{ picked: ['End the turn'] }])).toBeNull()
  })

  it('counts a whitespace-only Other as unanswered', () => {
    expect(formatAnswer(viewOf(APPROACH), [{ picked: [], other: '   ' }])).toBeNull()
  })

  it('ignores a picked label that is not one of the options', () => {
    expect(formatAnswer(viewOf(CHANNELS), [{ picked: ['Fax'] }])).toBeNull()
    expect(formatAnswer(viewOf(CHANNELS), [{ picked: ['Fax', 'Slack'] }])).toBe('Channels: Slack')
  })

  it('sends a skipped question as its header with the skipped marker', () => {
    expect(formatAnswer(viewOf(APPROACH), [{ picked: [], skipped: true }])).toBe(
      `Approach: ${SKIPPED_ANSWER}`,
    )
  })

  it('a skip wins over whatever the question still holds', () => {
    expect(
      formatAnswer(viewOf(CHANNELS), [{ picked: ['Slack'], other: 'Email', skipped: true }]),
    ).toBe('Channels: (skipped)')
  })

  it('a card with every question skipped still sends', () => {
    expect(
      formatAnswer(viewOf(APPROACH, CHANNELS), [
        { picked: [], skipped: true },
        { picked: [], skipped: true },
      ]),
    ).toBe('Approach: (skipped)\nChannels: (skipped)')
  })

  it('an unskipped question still blocks the send', () => {
    expect(
      formatAnswer(viewOf(APPROACH, CHANNELS), [{ picked: [], skipped: true }, { picked: [] }]),
    ).toBeNull()
  })
})

describe('DISMISS_TEXT', () => {
  it('is a notice, not an answer line: no header prefix', () => {
    expect(DISMISS_TEXT.startsWith('I dismissed')).toBe(true)
    expect(DISMISS_TEXT).not.toContain('Approach:')
  })
})

describe('settlementsOf', () => {
  it('maps each settled question to its outcome', () => {
    const reply = {
      settlements: [
        { question_id: 'call_ask_1', outcome: 'answered', settled_at: 10 },
        { question_id: 'call_ask_2', outcome: 'dismissed', settled_at: 11 },
      ],
    }
    const settled = settlementsOf(reply)
    expect(settled?.get('call_ask_1')).toBe('answered')
    expect(settled?.get('call_ask_2')).toBe('dismissed')
    expect(settled?.get('call_ask_3')).toBeUndefined()
  })

  it('an empty list is known: nothing settled', () => {
    expect(settlementsOf({ settlements: [] })?.size).toBe(0)
  })

  it('skips rows it cannot read and keeps the rest', () => {
    const reply = {
      settlements: [
        { question_id: 'call_ask_1', outcome: 'replied', settled_at: 10 },
        { outcome: 'answered', settled_at: 11 },
        'call_ask_3',
        { question_id: 'call_ask_4', outcome: 'answered', settled_at: 12 },
      ],
    }
    const settled = settlementsOf(reply)
    expect(settled?.size).toBe(1)
    expect(settled?.get('call_ask_4')).toBe('answered')
  })

  it('an unreadable reply is unknown, not "nothing settled"', () => {
    expect(settlementsOf(null)).toBeUndefined()
    expect(settlementsOf(undefined)).toBeUndefined()
    expect(settlementsOf({})).toBeUndefined()
    expect(settlementsOf({ settlements: 'none' })).toBeUndefined()
    expect(settlementsOf([])).toBeUndefined()
  })
})

describe('turnPositionOf', () => {
  it('reads the turn, step and queued texts of a verbose harness::status report', () => {
    const report = {
      session_id: 'sess_1',
      turn_id: 'turn_7',
      status: 'running',
      step: 3,
      turn_count: 1,
      queued: [
        {
          id: 'q_1',
          entry_id: 'e_q_1',
          message: {
            role: 'user',
            content: [
              { type: 'text', text: 'Approach: End the turn' },
              { type: 'image', data: '...' },
            ],
          },
          queued_at: 1,
        },
        { id: 'q_2', entry_id: 'e_q_2', message: { role: 'user', content: [] }, queued_at: 2 },
      ],
    }
    expect(turnPositionOf(report)).toStrictEqual({
      turnId: 'turn_7',
      step: 3,
      queuedTexts: ['Approach: End the turn', ''],
    })
  })

  it('a lean report without a queue reads as an empty queue', () => {
    const report = { session_id: 'sess_1', turn_id: 'turn_7', status: 'completed', step: 2, turn_count: 1 }
    expect(turnPositionOf(report)).toStrictEqual({ turnId: 'turn_7', step: 2, queuedTexts: [] })
  })

  it('a null report means the session has no turn record', () => {
    // status.rs `handle`: `Ok(None)` when `get_turn` finds no record.
    expect(turnPositionOf(null)).toBeNull()
  })

  it('a report without a usable turn_id is unknown, not "no record"', () => {
    expect(turnPositionOf({ session_id: 'sess_1' })).toBeUndefined()
    expect(turnPositionOf({ session_id: 'sess_1', turn_id: null })).toBeUndefined()
    expect(turnPositionOf({ session_id: 'sess_1', turn_id: '  ' })).toBeUndefined()
  })

  it('any other value is unknown', () => {
    expect(turnPositionOf(undefined)).toBeUndefined()
    expect(turnPositionOf('turn_7')).toBeUndefined()
    expect(turnPositionOf(['turn_7'])).toBeUndefined()
    expect(turnPositionOf(42)).toBeUndefined()
  })
})

describe('movedOn', () => {
  // viewOf() asks in turn_7.
  const view = viewOf(APPROACH)
  const at = (turnId: string): TurnPosition => ({ turnId, step: 1, queuedTexts: [] })

  it('is false while the asking turn is current or the read is unknown', () => {
    expect(movedOn(view, at('turn_7'))).toBe(false)
    expect(movedOn(view, undefined)).toBe(false)
  })

  it('is true once the session is on another turn or has no record', () => {
    expect(movedOn(view, at('turn_8'))).toBe(true)
    expect(movedOn(view, null)).toBe(true)
  })
})

describe('sendLanded', () => {
  const TEXT = 'Approach: End the turn'
  const at = (turnId: string, step: number, queuedTexts: string[] = []): TurnPosition => ({
    turnId,
    step,
    queuedTexts,
  })

  it('landed when the text is parked in the queue (sent mid-stream)', () => {
    expect(sendLanded(at('turn_7', 2), at('turn_7', 2, ['other', TEXT]), TEXT)).toBe(true)
    // Even without a position before the send.
    expect(sendLanded(undefined, at('turn_7', 2, [TEXT]), TEXT)).toBe(true)
  })

  it('landed when the session moved to another turn', () => {
    expect(sendLanded(at('turn_7', 2), at('turn_8', 0), TEXT)).toBe(true)
  })

  it('landed when the same turn stepped on (the queue drained into it)', () => {
    expect(sendLanded(at('turn_7', 2), at('turn_7', 3), TEXT)).toBe(true)
  })

  it('landed when a session without a record now has one', () => {
    expect(sendLanded(null, at('turn_1', 0), TEXT)).toBe(true)
  })

  it('did not land while nothing changed', () => {
    expect(sendLanded(at('turn_7', 2), at('turn_7', 2), TEXT)).toBe(false)
    expect(sendLanded(at('turn_7', 2), at('turn_7', 2, ['something else']), TEXT)).toBe(false)
  })

  it('proves nothing on an unknown read after, or no position before and no queue match', () => {
    expect(sendLanded(at('turn_7', 2), undefined, TEXT)).toBe(false)
    expect(sendLanded(at('turn_7', 2), null, TEXT)).toBe(false)
    expect(sendLanded(undefined, at('turn_8', 0), TEXT)).toBe(false)
  })
})

describe('askCardState', () => {
  it('is open before any send while nothing is settled', () => {
    expect(askCardState(undefined, 'idle')).toBe('open')
  })

  it('is sending after Send until the send lands', () => {
    expect(askCardState(undefined, 'sending')).toBe('sending')
  })

  it('is answered or dismissed once settled, whatever the send phase', () => {
    for (const phase of ['idle', 'sending', 'unconfirmed', 'failed'] as const) {
      expect(askCardState('answered', phase)).toBe('answered')
      expect(askCardState('dismissed', phase)).toBe('dismissed')
    }
  })

  it('is not sent when the reads ran out with nothing landed', () => {
    expect(askCardState(undefined, 'unconfirmed')).toBe('not-sent')
  })

  it('is failed when compose threw', () => {
    expect(askCardState(undefined, 'failed')).toBe('failed')
  })
})
