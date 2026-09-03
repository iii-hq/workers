import { describe, expect, it } from 'vitest'
import type {
  AssistantMessage,
  FunctionTriggerMessage,
  SystemMessage,
  ThoughtMessage,
  UserMessage,
} from '@/types/chat'
import { deriveTurnVisualState } from './turn-visual-state'

const user: UserMessage = {
  id: 'user-1',
  role: 'user',
  content: 'Continue',
  createdAt: 0,
}

describe('deriveTurnVisualState', () => {
  it('narrates the gap before the first visible output', () => {
    expect(deriveTurnVisualState([user], true)).toMatchObject({
      phase: 'waiting',
      showWaiting: true,
      turnKey: 'user-1',
      betweenSteps: false,
    })
  })

  it('keeps the post-thought gap visible instead of producing a blank tail', () => {
    const thought: ThoughtMessage = {
      id: 'thought-1',
      role: 'thought',
      content: 'Inspecting',
      durationMs: 120,
      streaming: false,
      createdAt: 1,
    }

    expect(deriveTurnVisualState([user, thought], true)).toMatchObject({
      phase: 'waiting',
      showWaiting: true,
      betweenSteps: true,
    })
  })

  it('distinguishes live calls from their between-step result dwell', () => {
    const call: FunctionTriggerMessage = {
      id: 'call-1',
      role: 'function-trigger',
      functionId: 'shell::run',
      input: {},
      running: true,
      createdAt: 1,
    }

    expect(deriveTurnVisualState([user, call], true).phase).toBe('calling')
    expect(
      deriveTurnVisualState(
        [user, { ...call, running: false, output: { ok: true } }],
        true,
      ),
    ).toMatchObject({ phase: 'waiting', showWaiting: true, betweenSteps: true })
  })

  it('waits after an intermediate assistant update but not final prose', () => {
    const progress: AssistantMessage = {
      id: 'assistant-1',
      role: 'assistant',
      content: 'The first phase is complete.',
      stopReason: 'function_call',
      createdAt: 1,
    }

    expect(deriveTurnVisualState([user, progress], true).showWaiting).toBe(true)
    expect(
      deriveTurnVisualState([user, { ...progress, stopReason: 'end' }], true),
    ).toMatchObject({ phase: 'answering', showWaiting: false })
  })

  it('turns every visual activity off when the session is idle', () => {
    expect(deriveTurnVisualState([user], false)).toMatchObject({
      phase: 'idle',
      showWaiting: false,
    })
  })

  it('uses each trigger notification as a new turn clock identity', () => {
    const notification: UserMessage = {
      ...user,
      id: 'e_fire_sub_1_4',
      notification: true,
    }

    expect(deriveTurnVisualState([notification], true)).toMatchObject({
      phase: 'waiting',
      turnKey: 'trigger:fire:sub_1_4',
    })
  })

  it('keeps one clock identity when the trigger record arrives first', () => {
    const record: SystemMessage = {
      id: 'e_trigfired_sub_1_4',
      role: 'system',
      content: '',
      kind: 'trigger-fired',
      createdAt: 0,
      trigger: {
        subscription_id: 'sub_1',
        target: 'harness::send',
        once: false,
        retired: false,
        fired_at: 0,
      },
    }
    const notification: UserMessage = {
      ...user,
      id: 'e_fire_sub_1_4',
      notification: true,
    }

    expect(deriveTurnVisualState([record], true)).toMatchObject({
      phase: 'waiting',
      turnKey: 'trigger:fire:sub_1_4',
      betweenSteps: false,
    })
    expect(deriveTurnVisualState([record, notification], true).turnKey).toBe(
      'trigger:fire:sub_1_4',
    )
  })
})
