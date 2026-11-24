import { describe, expect, it } from 'vitest'
import { observerNoticeHeadline } from './observer-notice'

describe('observerNoticeHeadline', () => {
  it('names what stopped being observed, never "wake" or "binding"', () => {
    const cases: [string, string][] = [
      [
        '[notification] wake expired unfired: `state::on-changed` filters {"input.key":{"$eq":"child-a"}} (binding sub_1) passed its lifecycle deadline (expires_at 1) with 0 fires. Nothing else will wake this session for it — re-arm a new watch …',
        'Stopped observing state::on-changed · expired before it fired',
      ],
      [
        '[notification] wake removed: `kanban::on-change` (binding sub_2) was unobserved by session s_1 before it ever fired.',
        'Stopped observing kanban::on-change · removed before it fired',
      ],
      [
        '[notification] binding sub_3 (was observing `harness::on-turn-end`) was RETIRED: `harness::spawn` is not a binding target',
        'Stopped observing harness::on-turn-end',
      ],
      [
        '[notification] binding sub_4 was RETIRED: `harness::spawn` is not a binding target',
        'Observer retired',
      ],
      [
        '[notification] binding sub_5 (observing `state::on-changed`) is DROPPING fires: the delivery claim failed',
        'Observing state::on-changed · events are being dropped',
      ],
      [
        '[notification] observer obs_1 fired but was NOT delivered: condition failed',
        'Observer fired but was not delivered',
      ],
      [
        '[notification] wake expired unfired: unknown watch (binding sub_6) passed',
        'Observer expired before it fired',
      ],
    ]
    for (const [text, headline] of cases) {
      expect(observerNoticeHeadline(text)).toBe(headline)
      expect(headline).not.toMatch(/\b(wakes?|arm(ed|ing)?|binding|tools?)\b/i)
    }
  })

  it('leaves delivered events and typed text alone', () => {
    expect(
      observerNoticeHeadline('[notification] child-a-done: {"key":"child-a"}'),
    ).toBeUndefined()
    expect(
      observerNoticeHeadline('wake me when the build is done'),
    ).toBeUndefined()
  })
})
