import { type ISdk, registerWorker } from 'iii-browser-sdk'
import {
  expect,
  expectPassingResult,
  type HarnessStack,
  openSession,
  test,
} from './harness-stack'

/*
 * chat-projects §4.3 (phase 4): Claude Code accounts in the Chat, against
 * an isolated stack the fixture starts. tmux and the phase-4 harness
 * functions are fakes registered here with iii-browser-sdk:
 * `tmux::accounts::{list,credential,global}`, `tmux::usage::read`,
 * `harness::retry` and `harness::metrics::account-split`. The failed turn is
 * a fake too: a harness `error` record with `code: "llm.usage_limit"` and
 * its details, appended to the session after the scenario's real turn, on a
 * conversation whose model is `claude-code::…` (a real one needs the
 * provider and the harness's phase 4). Nothing reaches Anthropic or a real
 * account.
 */

const HOUR = 3_600_000
const NOW = Date.now()
const CLAUDE_MODEL = 'claude-code::claude-opus-4-5'

interface PendingGlobal {
  account: string
  by: string
  waiting_for: string[]
  at_ms: number
}

interface Fakes {
  calls: { functionId: string; payload: Record<string, unknown> }[]
  global: string
  /** The next global switch waits for windows pinned to its account. */
  waitNext: boolean
  pending: PendingGlobal | null
  dispose: () => Promise<void>
}

function usage(global: string, pending: PendingGlobal | null = null) {
  return {
    generated_at_ms: Date.now() - 4 * 60_000,
    claude: {
      available: true,
      managed: true,
      global,
      switch_delay_s: 0,
      accounts: [
        {
          id: 'default',
          name: 'Work',
          color: 'blue',
          is_default: true,
          email: 'me@acme.dev',
          plan: 'team',
          signed_in: true,
          token_expired: false,
          dir_display: '~/.claude',
          panes: 2,
          usage: {
            five_hour: { used_percent: 38, resets_at_ms: NOW + 2 * HOUR },
            week: { used_percent: 64, resets_at_ms: NOW + 50 * HOUR },
            as_of_ms: NOW - 4 * 60_000,
          },
        },
        {
          id: 'personal',
          name: 'Personal',
          color: 'violet',
          is_default: false,
          email: 'me@home.dev',
          plan: 'max',
          tier: 'default_claude_max_20x',
          signed_in: true,
          token_expired: false,
          dir_display: '~/.local/share/iii/claude-accounts/personal',
          panes: 1,
          usage: {
            five_hour: { used_percent: 100, resets_at_ms: NOW + HOUR },
            week: { used_percent: 22, resets_at_ms: NOW + 90 * HOUR },
            as_of_ms: NOW - 6 * 60_000,
          },
        },
        {
          id: 'side-project',
          name: 'Side project',
          color: 'green',
          is_default: false,
          signed_in: false,
          token_expired: false,
          dir_display: '~/.local/share/iii/claude-accounts/side-project',
          panes: 0,
        },
      ],
      rules: {
        mode: 'suggest',
        at: 'week_90',
        five_hour: true,
        back_after_reset: false,
        order: [],
      },
      suggestion: null,
      pending_global: pending,
      logins: [],
    },
  }
}

const SPLITS = [
  {
    account: 'default',
    week_percent: 64,
    resets_at_ms: NOW + 50 * HOUR,
    outside_measured: true,
    segments: [
      {
        kind: 'project',
        project_id: 'prj_acme_web',
        name: 'acme-web',
        color: 'blue',
        percent: 41,
      },
      { kind: 'no_project', percent: 9 },
      { kind: 'outside', percent: 14 },
    ],
  },
  {
    account: 'personal',
    week_percent: 22,
    resets_at_ms: NOW + 90 * HOUR,
    outside_measured: false,
    segments: [{ kind: 'no_project', percent: 22 }],
  },
]

/** tmux and the phase-4 harness functions, as fakes that record their calls. */
function registerFakes(engineUrl: string): Fakes {
  const sdk: ISdk = registerWorker(engineUrl)
  let disposed = false
  const fakes: Fakes = {
    calls: [],
    global: 'personal',
    waitNext: false,
    pending: null,
    dispose: async () => {
      if (disposed) return
      disposed = true
      for (const ref of refs) ref.unregister()
      await sdk.shutdown().catch(() => undefined)
    },
  }
  const fake = (
    functionId: string,
    answer: (payload: Record<string, unknown>) => unknown,
  ) =>
    sdk.registerFunction(functionId, async (payload) => {
      const input = (payload ?? {}) as Record<string, unknown>
      fakes.calls.push({ functionId, payload: input })
      return answer(input)
    })
  const refs = [
    fake(
      'tmux::accounts::list',
      () => usage(fakes.global, fakes.pending).claude,
    ),
    fake('tmux::accounts::credential', () => ({ error: 'fake' })),
    fake('tmux::usage::read', () => usage(fakes.global, fakes.pending)),
    fake('tmux::accounts::global', (input) => {
      if (fakes.waitNext) {
        // tmux keeps the old global and records the switch it waits for.
        fakes.waitNext = false
        fakes.pending = {
          account: String(input.account),
          by: 'person',
          waiting_for: ['%7'],
          at_ms: Date.now(),
        }
        return {
          global: fakes.global,
          state: 'waiting',
          following: 1,
          restarted: [],
          after_turn: [],
          waiting_for: ['%7'],
        }
      }
      fakes.global = String(input.account)
      return {
        global: fakes.global,
        state: 'switched',
        following: 1,
        restarted: [],
        after_turn: [],
        waiting_for: [],
      }
    }),
    fake('harness::metrics::account-split', () => SPLITS),
    fake('harness::retry', () => ({ turn_id: 'fake-retry' })),
  ]
  return fakes
}

async function sessionMetadata(
  stack: HarnessStack,
): Promise<Record<string, unknown>> {
  const got = (await stack.invoke('session::get', {
    session_id: stack.ready.session.id,
  })) as { meta?: { metadata?: Record<string, unknown> } } | null
  return got?.meta?.metadata ?? {}
}

const shot = (name: string) =>
  test.info().outputPath(`claude-accounts-${name}.png`)

test.describe('Claude Code accounts in the Chat (§4.3)', () => {
  test.use({ scenario: 'streamed-text' })

  test('the limit card, the account chip and row, the screen and a project pin', async ({
    page,
    stack,
  }) => {
    const fakes = registerFakes(stack.ready.engine_url)
    try {
      const completed = stack.waitForTurnCompleted()
      await stack.send()
      expect(await completed).toMatchObject({ status: 'completed' })

      const sessionId = stack.ready.session.id
      // A Claude Code conversation whose next turn hit Personal's 5-hour limit.
      await stack.invoke('session::patch-meta', {
        session_id: sessionId,
        set: { model: CLAUDE_MODEL },
      })
      await stack.invoke('session::append', {
        session_id: sessionId,
        entry_id: 'e-fake-usage-limit',
        custom: {
          custom_type: 'error',
          data: {
            status: 'error',
            code: 'llm.usage_limit',
            class: 'llm.usage_limit',
            summary: 'Claude Code usage limit reached.',
            provider: 'claude-code',
            details: {
              usage_limit: true,
              window: 'five_hour',
              resets_at_ms: NOW + HOUR,
              account: 'personal',
            },
            timestamp: Date.now(),
          },
        },
      })
      const project = (await stack.invoke('harness::projects::create', {
        name: 'acme-web',
        color: 'blue',
        folders: ['/srv/acme-web'],
      })) as { project: { id: string } }

      await page.setViewportSize({ width: 1280, height: 860 })
      await openSession(page, stack)

      /* The limit card in place of "Turn ended · Continue" (§4.3.4). */
      const card = page.locator('[data-limit-card]')
      await expect(card).toHaveAttribute('data-limit-card', 'failed')
      await expect(card.getByRole('heading')).toHaveText(
        'Personal reached its 5-hour limit',
      )
      await expect(card).toContainText('Resets')
      await expect(card).toContainText(
        'Changing account starts a new prompt cache for this conversation.',
      )
      // Personal is the global one and the conversation follows it.
      await expect(card.locator('[data-limit-make-global]')).toHaveText(
        'Make Work the global',
      )
      // The transcript says it's the plan's limit, not a billing failure.
      await expect(
        page.getByText('This Claude Code account reached its usage limit'),
      ).toBeVisible()
      // Its steps still hold once another account has taken over.
      await expect(
        page.locator('[data-message-role="turn-failure"]'),
      ).toContainText(
        'Another account can take over from the limit card above the composer.',
      )
      // The chip in the header says where the conversation runs.
      const chip = page.locator('[data-account-chip]')
      await expect(chip).toHaveText('Personal')
      await page.screenshot({ path: shot('limit-card') })

      // Other… lists the other accounts with their use; the signed-out one is off.
      await card.locator('[data-limit-other]').click()
      const other = page.getByRole('menu')
      await expect(
        other.locator('[data-limit-account="default"]'),
      ).toContainText('5 hours 38% · This week 64%')
      await expect(
        other.locator('[data-limit-account="side-project"]'),
      ).toHaveAttribute('data-disabled', '')
      await page.keyboard.press('Escape')

      // "Make Work the global" while a window pinned to Work is busy: tmux
      // waits, so the turn doesn't run again yet, and "Continue in" stays.
      fakes.waitNext = true
      await card.locator('[data-limit-make-global]').click()
      await expect(card.locator('[data-limit-waiting]')).toHaveText(
        'Switching to Work when 1 window pinned to it finishes its turn.',
      )
      await expect(card.locator('[data-limit-make-global]')).toBeDisabled()
      await expect(
        card.locator('[data-limit-continue="default"]'),
      ).toBeEnabled()
      expect(
        fakes.calls
          .filter((c) => c.functionId === 'tmux::accounts::global')
          .map((c) => c.payload.account),
      ).toEqual(['default'])
      expect(
        fakes.calls.filter((c) => c.functionId === 'harness::retry'),
      ).toEqual([])
      await card.screenshot({ path: shot('limit-card-waiting') })

      // "Continue in Work": pins the conversation, then re-runs the turn.
      await card.locator('[data-limit-continue="default"]').click()
      await expect
        .poll(() =>
          fakes.calls
            .filter((c) => c.functionId === 'harness::retry')
            .map((c) => c.payload.session_id),
        )
        .toEqual([sessionId])
      const meta = await sessionMetadata(stack)
      expect(meta.claude_account).toBe('default')
      // The account alone was written: the model stays.
      expect(meta.model).toBe(CLAUDE_MODEL)
      await expect(chip).toHaveText('Work')
      // Until the re-run turn changes the card, nothing more to press.
      await expect(card.locator('[data-limit-continue]')).toBeDisabled()
      await expect(card.locator('[data-limit-other]')).toBeDisabled()
      // The X puts it away (the fake retry ran no turn, so it is still there).
      await card.locator('[data-limit-dismiss]').click()
      await expect(card).toHaveCount(0)

      /* The chip opens Conversation details on the account row (§4.3.5). */
      await chip.click()
      const details = page.getByRole('dialog', { name: 'Conversation details' })
      await expect(details.locator('[data-account-resolution]')).toHaveText(
        'Runs on Work, pinned for this conversation.',
      )
      await expect(
        details.locator('[data-conversation-account] [role="combobox"]'),
      ).toBeFocused()
      await details.screenshot({ path: shot('details-row') })
      await page.keyboard.press('Escape')
      await expect(details).toHaveCount(0)
      // The card put away, "Turn ended · Continue" doesn't come in its place:
      // it would run the same turn into the same limit.
      await expect(page.getByText('Turn ended.')).toHaveCount(0)
      await expect(
        page.getByRole('button', { name: 'Continue', exact: true }),
      ).toHaveCount(0)

      /* The sidebar footer opens the screen (§4.3.1, §4.3.3): one button,
         the Claude Code mark and a ring per account, the global (Personal)
         first, its words in the button's name and each ring's tooltip (D178). */
      const footer = page.locator('[data-usage-footer]')
      const rings = footer.getByRole('button', {
        name: 'Claude Code usage this week: Personal, global, 22%; Work, 64%; Side project, Not signed in. Opens Claude Code accounts.',
      })
      await expect(rings).toBeVisible()
      await expect(rings).toHaveText('')
      await expect(rings.locator('[data-footer-account]')).toHaveCount(3)
      expect(
        await rings
          .locator('[data-footer-account]')
          .evaluateAll((els) =>
            els.map((el) => el.getAttribute('data-footer-account')),
          ),
      ).toEqual(['personal', 'default', 'side-project'])
      await rings.locator('[data-footer-account="personal"]').hover()
      await expect(page.locator('[data-slot="tooltip-content"]')).toHaveText(
        'Personal · global · 22%',
      )
      await footer.screenshot({ path: shot('footer-rings') })
      await rings.click()
      const screen = page.getByRole('dialog', { name: 'Claude Code accounts' })
      await expect(screen).toBeVisible()
      const work = screen.locator('[data-account-card="default"]')
      await expect(work).toContainText('Team')
      await expect(work.locator('[data-split-item]')).toHaveText([
        'acme-web 41%',
        'no project 9%',
        'Outside the Chat 14%',
      ])
      await expect(
        screen.locator('[data-account-card="personal"] [data-split-note]'),
      ).toHaveText("Usage outside the Chat isn't measured here.")
      await expect(
        screen.locator('[data-account-card="side-project"]'),
      ).toContainText('Not signed in')
      await screen.screenshot({ path: shot('screen-accounts') })

      /* The Projects tab: a row opens the project on its account (§4.3.2). */
      await screen.getByRole('tab', { name: 'Projects' }).click()
      const row = screen.getByRole('row', { name: "Edit acme-web's account" })
      await expect(row).toContainText('Follow global (Personal)')
      await screen.screenshot({ path: shot('screen-projects') })
      await row.click()
      const edit = page.getByRole('dialog', { name: 'Edit project' })
      const section = edit.locator('[data-project-account]')
      await expect(section).toContainText('Personal now')
      await section.getByText('Pin to', { exact: true }).click()
      // The pin starts on the account with the most room: Work.
      await expect(section.locator('[data-project-account-pin]')).toHaveText(
        'Work · 64% this week',
      )
      await edit.screenshot({ path: shot('project-account') })
      await edit.getByRole('button', { name: 'Save', exact: true }).click()
      await expect(edit).toHaveCount(0)
      await expect
        .poll(async () => {
          const list = (await stack.invoke('harness::projects::list', {
            version: 2,
          })) as { projects: { id: string; account?: unknown }[] }
          return list.projects.find((p) => p.id === project.project.id)?.account
        })
        .toEqual({ mode: 'pinned', account_id: 'default' })
      await expect(row).toContainText('Work (pinned)')

      await fakes.dispose()
      expectPassingResult(await stack.finish())
    } finally {
      await fakes.dispose()
    }
  })
})
