import type { Locator, Page } from '@playwright/test'
import { expect, openSession, test } from './harness-stack'

/*
 * chat-projects §1.3.6 on a phone (390x844): the context drawer's filter
 * chips, and "Show in conversation" from a row's ⋮.
 *
 * Hooks a reviewer can read too:
 * - the drawer: [data-side-panel-drawer], the dialog named "Context";
 * - the chips: the toolbar named "Show" ([data-context-filter] holds the
 *   pressed one), one button per non-empty section, [data-context-chip="all"
 *   | subagents | prs | links | attachments | images] with aria-pressed (a
 *   section's own header reads the same "Links 2", so match chips by id);
 * - the sections: regions named by their label ("Pull requests", "Links").
 *
 * The conversation is the playground scenario's session, given a first
 * message that names a PR and a page, then enough messages to push it out
 * of sight; harness::context-items lists the PR (mentioned) and the links.
 */
const PR = 'https://github.com/acme/web/pull/7'
const DOCS = 'https://example.com/docs/validation'

test.describe('the context drawer on a phone', () => {
  test.use({ scenario: 'console-set-description' })

  test('chips filter the sections, and Show in conversation closes the drawer before landing', async ({
    page,
    stack,
  }) => {
    const session = stack.ready.session.id
    // User messages: an assistant entry needs the provider's fields.
    const append = (text: string) =>
      stack.invoke('session::append', {
        session_id: session,
        message: {
          role: 'user',
          content: [{ type: 'text', text }],
          timestamp: Date.now(),
        },
      })
    await append(
      `The login form rejects plus addresses. See ${PR} and ${DOCS}.`,
    )
    for (let i = 0; i < 24; i += 1) {
      await append(
        `Step ${i + 1}: ${'a long line of reasoning about the validator '.repeat(6)}`,
      )
    }

    await page.setViewportSize({ width: 1280, height: 800 })
    await openSession(page, stack)
    await page.setViewportSize({ width: 390, height: 844 })
    await page.emulateMedia({ reducedMotion: 'no-preference' })

    const button = page.locator('[data-context-pane-button]')
    const drawer = page.locator('[data-side-panel-drawer]')
    await expect(button).toHaveAttribute('aria-label', 'Show context pane')
    await button.click()
    await expect(drawer).toBeVisible()
    await expect(drawer).toHaveAttribute('aria-label', 'Context')

    const chips = drawer.getByRole('toolbar', { name: 'Show' })
    // A chip by its id; the section's own header also reads "Links 2", so
    // never pick a chip by its text alone.
    const chip = (id: string) => chips.locator(`[data-context-chip="${id}"]`)
    const region = (name: string): Locator =>
      drawer.getByRole('region', { name, exact: true })
    const sections = { prs: 'Pull requests', links: 'Links' } as const

    // All: every non-empty section, All pressed.
    await expect(chips).toHaveAttribute('data-context-filter', 'all')
    await expect(chip('all')).toHaveAttribute('aria-pressed', 'true')
    for (const label of Object.values(sections))
      await expect(region(label)).toBeVisible()
    await expect(chip('prs')).toHaveText('PRs1')

    // Each chip shows only its section, and says it is pressed.
    for (const [id, label] of Object.entries(sections)) {
      await chip(id).click()
      await expect(chip(id)).toHaveAttribute('aria-pressed', 'true')
      await expect(chip('all')).toHaveAttribute('aria-pressed', 'false')
      await expect(chips).toHaveAttribute('data-context-filter', id)
      await expect(region(label)).toBeVisible()
      for (const other of Object.values(sections).filter((l) => l !== label))
        await expect(region(other)).toHaveCount(0)
      expect(await drawer.locator('section[data-reorder-item]').count()).toBe(1)
    }
    await chip('all').click()
    await expect(chip('all')).toHaveAttribute('aria-pressed', 'true')
    await expect(chips).toHaveAttribute('data-context-filter', 'all')
    for (const label of Object.values(sections))
      await expect(region(label)).toBeVisible()

    // Show in conversation from the PR's ⋮: the drawer closes, then lands.
    const target = page.locator('[data-message-row]', { hasText: PR })
    await region('Pull requests')
      .getByRole('button', { name: 'Actions for acme/web#7' })
      .click()
    await sampleUntilLanded(page, async () => {
      await page.getByRole('menuitem', { name: 'Show in conversation' }).click()
    })
    const samples = await landingSamples(page)
    const landed = samples.findIndex((s) => s.flash)
    expect(landed, JSON.stringify(samples)).toBeGreaterThan(0)
    // The first frame with the flash already has the drawer gone and the
    // button back to Show.
    expect(samples[landed]).toMatchObject({
      drawer: 'gone',
      label: 'Show context pane',
    })
    await expect(drawer).toHaveCount(0)
    await expect(button).toHaveAttribute('aria-label', 'Show context pane')
    await expect(target).toHaveAttribute('data-flash', 'true')
    await expect(target).toBeInViewport()
    // The 1.2 s flash ends.
    await expect(target).not.toHaveAttribute('data-flash', 'true', {
      timeout: 3000,
    })
  })
})

interface LandingSample {
  t: number
  drawer: 'open' | 'ending' | 'gone'
  flash: boolean
  label: string | null
}

/** Sample every frame from just before `act` until the row has flashed. */
async function sampleUntilLanded(page: Page, act: () => Promise<void>) {
  await page.evaluate(() => {
    const w = window as typeof window & { __landing?: unknown[] }
    const samples: unknown[] = []
    w.__landing = samples
    const t0 = performance.now()
    const frame = () => {
      const drawer = document.querySelector('[data-side-panel-drawer]')
      samples.push({
        t: Math.round(performance.now() - t0),
        drawer: drawer
          ? drawer.hasAttribute('data-ending-style')
            ? 'ending'
            : 'open'
          : 'gone',
        flash:
          document.querySelector('[data-message-row][data-flash="true"]') !==
          null,
        label:
          document
            .querySelector('[data-context-pane-button]')
            ?.getAttribute('aria-label') ?? null,
      })
      if (performance.now() - t0 < 4000) requestAnimationFrame(frame)
    }
    requestAnimationFrame(frame)
  })
  await act()
  await expect
    .poll(async () => (await landingSamples(page)).some((s) => s.flash), {
      timeout: 4000,
    })
    .toBe(true)
}

async function landingSamples(page: Page): Promise<LandingSample[]> {
  return page.evaluate(
    () =>
      ((window as typeof window & { __landing?: unknown[] }).__landing ??
        []) as LandingSample[],
  )
}
