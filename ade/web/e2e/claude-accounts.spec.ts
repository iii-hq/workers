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
 * the isolated stack of UI-008 (`console-account-usage-limit`), whose
 * harness is this branch's: the account resolution, `harness::retry`, the
 * usage ledger and `harness::metrics::account-split` are real, and so are
 * the turns, on the scripted `claude-code` model `claude-opus-4-5`. The
 * conversation is in the project `acme-web` (the scenario's setup call),
 * which follows the global account.
 *
 * Only tmux is fake (the stack runs no tmux worker): its account functions
 * are registered here with iii-browser-sdk before the send —
 * `tmux::accounts::{list,credential,global}`, `tmux::usage::read` and
 * `tmux::usage::claude_outside` — offering Work (`default`), Personal
 * (`personal`, the global) and a signed-out Side project. So the turn runs
 * on Personal and the scripted router answers its five-hour usage limit;
 * "Continue in Work" pins the conversation and the real retry runs the turn
 * again on Work, where the router answers. Nothing reaches Anthropic or a
 * real account.
 */

const HOUR = 3_600_000
const NOW = Date.now()
/** The playground's session model (UI-008's `Model::claude_code`). */
const CLAUDE_MODEL = 'claude-code::claude-opus-4-5'
/** UI-008's reply on Work, after the retry. */
const REPLY = 'The release adds account pinning.'

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

/**
 * What tmux measured of Work's week outside the Chat: a third of the
 * retried turn's tokens (30 in, 6 out) on the same model, so the real split
 * gives acme-web three quarters of Work's 64% and the outside a quarter,
 * whatever the model's price. Personal's outside isn't measured.
 */
const OUTSIDE_BY_MODEL = [{ model: 'claude-opus-4-5', input: 10, output: 2 }]

/** tmux, as fakes that record their calls. */
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
    fake('tmux::usage::claude_outside', (input) => {
      if (input.account !== 'default') throw new Error('not measured (fake)')
      return {
        account: 'default',
        since_ms: Number(input.since_ms ?? 0),
        as_of_ms: Date.now(),
        attributed: 0,
        by_model: OUTSIDE_BY_MODEL,
      }
    }),
  ]
  return fakes
}

/** The fakes are on the engine: the harness resolves the turn's account with them. */
async function fakesRegistered(stack: HarnessStack): Promise<void> {
  await expect
    .poll(
      async () => {
        const list = (await stack
          .invoke('tmux::accounts::list', {})
          .catch(() => null)) as { global?: string } | null
        return list?.global ?? null
      },
      { timeout: 15_000 },
    )
    .toBe('personal')
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
  // UTC: the card's reset line reads the scenario's reset as a date.
  test.use({ scenario: 'console-account-usage-limit', timezoneId: 'UTC' })

  test('the limit card, the account chip and row, the screen and a project pin', async ({
    page,
    stack,
  }) => {
    const fakes = registerFakes(stack.ready.engine_url)
    try {
      await fakesRegistered(stack)
      const sessionId = stack.ready.session.id
      // The turn runs on the global, Personal, and hits its 5-hour limit.
      const failed = stack.waitForTurnCompleted()
      await stack.send()
      expect(await failed).toMatchObject({ status: 'failed' })
      const projects = (await stack.invoke('harness::projects::list', {
        version: 2,
      })) as { projects: { id: string; name: string }[] }
      const project = projects.projects.find((p) => p.name === 'acme-web')
      expect(project).toBeDefined()
      const joined = await sessionMetadata(stack)
      // The send's folder filed it into the project (D8); it follows the global.
      expect(joined.project_id).toBe(project?.id)
      expect(joined.claude_account).toBeUndefined()
      expect(joined.model).toBe(CLAUDE_MODEL)

      await page.setViewportSize({ width: 1280, height: 860 })
      await openSession(page, stack)

      /* The limit card above the composer (§4.3.4). */
      const card = page.locator('[data-limit-card]')
      await expect(card).toHaveAttribute('data-limit-card', 'failed')
      await expect(card.getByRole('heading')).toHaveText(
        'Personal reached its 5-hour limit',
      )
      // The window and the reset come from the failure (UI-008's router).
      await expect(
        card.getByText('Resets Jan 1', { exact: true }),
      ).toBeVisible()
      await expect(card).toContainText(
        'Changing account starts a new prompt cache for this conversation.',
      )
      // Personal is the global one and the conversation follows it.
      await expect(card.locator('[data-limit-make-global]')).toHaveText(
        'Make Work the global',
      )
      // The transcript says it's the plan's limit, not a billing failure.
      const failure = page.locator('[data-message-role="turn-failure"]')
      await expect(
        failure.getByRole('heading', {
          name: 'This Claude Code account reached its usage limit',
          exact: true,
        }),
      ).toBeVisible()
      // Its steps still hold once another account has taken over.
      await expect(failure).toContainText(
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
      // No turn ran again: the last one is still the failed one, on Personal.
      expect(
        await stack.invoke('harness::status', {
          session_id: sessionId,
          verbose: true,
        }),
      ).toMatchObject({
        status: 'failed',
        account: { id: 'personal', source: 'global' },
        failure: { code: 'llm.usage_limit' },
      })
      await card.screenshot({ path: shot('limit-card-waiting') })

      // The X puts the card away.
      await card.locator('[data-limit-dismiss]').click()
      await expect(card).toHaveCount(0)
      // Put away in this tab only: opened again, the conversation shows it.
      await openSession(page, stack)
      await expect(card).toHaveAttribute('data-limit-card', 'failed')

      // "Continue in Work": pins the conversation, then re-runs the turn
      // (the real `harness::retry`), which the router answers on Work.
      await card.locator('[data-limit-continue="default"]').click()
      await expect(page.getByText(REPLY, { exact: true })).toBeVisible()
      // The re-run turn took the card away.
      await expect(card).toHaveCount(0)
      await expect
        .poll(() =>
          stack.invoke('harness::status', {
            session_id: sessionId,
            verbose: true,
          }),
        )
        .toMatchObject({
          status: 'completed',
          account: { id: 'default', source: 'session' },
        })
      const meta = await sessionMetadata(stack)
      expect(meta.claude_account).toBe('default')
      // The account alone was written: the model stays.
      expect(meta.model).toBe(CLAUDE_MODEL)
      await expect(chip).toHaveText('Work')

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
      // The turn ran again: its reply stays and the card does not come back.
      await expect(page.getByText(REPLY, { exact: true })).toBeVisible()
      await expect(card).toHaveCount(0)

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
      // The real split: Work's 64% between the retried turn in acme-web
      // (from the harness's ledger) and the outside (the fake tmux), 3 to 1.
      await expect(work.locator('[data-split-item]')).toHaveText([
        'acme-web 48%',
        'Outside the Chat 16%',
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
          return list.projects.find((p) => p.id === project?.id)?.account
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
