import type { FunctionTriggerMessage } from '@iii-dev/console-ui'
import { describe, expect, it } from 'vitest'
import {
  type AskQuestionView,
  type AskView,
  formatAnswer,
  isOpen,
  parseAsk,
  turnIdOf,
} from './ask'

/**
 * The row the console hands a renderer: the host contract plus the extra
 * fields ade/web's entry-mapper sets on it (`functionTriggerId`,
 * `sessionId`, `resultEntryId`, `unloaded`).
 */
type ConsoleRow = FunctionTriggerMessage & {
  functionTriggerId?: string
  sessionId?: string
  resultEntryId?: string
  unloaded?: boolean
}

/** The model's arguments (the `payload` inside agent_trigger). */
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

/** A settled row for an agent_trigger-wrapped call: `unwrapFunctionTrigger`
 *  lifts `function`/`payload`/`description`; the result pairing sets
 *  `output`, `running: false`, `pendingApproval: false`, `resultEntryId`. */
function wrappedRow(output: unknown): ConsoleRow {
  return {
    id: 'e_asst_3:1',
    role: 'function-trigger',
    functionId: 'harness::ask',
    description: 'Ask which approach to take',
    input: REQUEST,
    output,
    running: false,
    pendingApproval: false,
    functionTriggerId: 'call_ask_1',
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
  it('builds the view from an accepted ask wrapped in agent_trigger', () => {
    expect(parseAsk(wrappedRow(okOutput(DETAILS)))).toStrictEqual(VIEW)
  })

  it('builds the view from a native call (input is the raw arguments)', () => {
    const row: ConsoleRow = {
      id: 'e_asst_3:0',
      role: 'function-trigger',
      functionId: 'harness::ask',
      input: REQUEST,
      output: okOutput(DETAILS),
      running: false,
      pendingApproval: false,
      functionTriggerId: 'call_ask_1',
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
})

describe('isOpen', () => {
  const view = viewOf(APPROACH)

  it('is open while the current turn is the one that asked', () => {
    expect(isOpen(view, 'turn_7')).toBe(true)
  })

  it('is closed once the session moved to another turn', () => {
    expect(isOpen(view, 'turn_8')).toBe(false)
  })

  it('is closed when the session has no turn record (expired or none)', () => {
    expect(isOpen(view, null)).toBe(false)
  })

  it('is open when the current turn is unknown (not read yet, or the read failed)', () => {
    expect(isOpen(view, undefined)).toBe(true)
  })
})

describe('turnIdOf', () => {
  it('reads the turn id of a lean harness::status report', () => {
    const report = { session_id: 'sess_1', turn_id: 'turn_7', status: 'completed', step: 2, turn_count: 1 }
    expect(turnIdOf(report)).toBe('turn_7')
  })

  it('a null report means the session has no turn record', () => {
    // status.rs `handle`: `Ok(None)` when `get_turn` finds no record.
    expect(turnIdOf(null)).toBeNull()
  })

  it('a report without a usable turn_id is unknown, not "no record"', () => {
    // A report exists only when a record does, and `handle` always fills
    // `turn_id` then; a report we cannot read proves nothing either way.
    expect(turnIdOf({ session_id: 'sess_1' })).toBeUndefined()
    expect(turnIdOf({ session_id: 'sess_1', turn_id: null })).toBeUndefined()
    expect(turnIdOf({ session_id: 'sess_1', turn_id: '  ' })).toBeUndefined()
  })

  it('any other value is unknown', () => {
    expect(turnIdOf(undefined)).toBeUndefined()
    expect(turnIdOf('turn_7')).toBeUndefined()
    expect(turnIdOf(['turn_7'])).toBeUndefined()
    expect(turnIdOf(42)).toBeUndefined()
  })
})
