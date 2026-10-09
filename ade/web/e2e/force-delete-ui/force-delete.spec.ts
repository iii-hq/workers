import { expect, test } from '@playwright/test'

async function outcomeCounts(
  dialog: ReturnType<import('@playwright/test').Page['getByRole']>,
) {
  return dialog
    .locator('[data-deletion-outcome-counts] > div')
    .evaluateAll((nodes) =>
      nodes
        .map(
          (node) =>
            `${node.querySelector('dt')?.textContent}: ${node.querySelector('dd')?.textContent}.`,
        )
        .join(' '),
    )
}

for (const theme of ['light', 'dark']) {
  for (const [name, width, pane] of [
    ['phone', 390, 390],
    ['narrow', 1280, 420],
    ['wide', 1280, 1280],
  ] as const) {
    test(`${name} ${theme}: stale parent overlap permits only explicit normal retry`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 })
      await page.goto(`/?theme=${theme}&pane=${pane}&scenario=overlap-missing`)
      if (name !== 'wide')
        await page
          .getByRole('button', { name: 'Back to conversations' })
          .click()
      await page.getByRole('button', { name: 'Request removal' }).click()
      const dialog = page.getByRole('dialog')
      await dialog.getByRole('button', { name: 'Delete', exact: true }).click()
      await dialog
        .getByRole('button', { name: 'Review existing deletion' })
        .click()
      await expect(dialog).toContainText('unavailable in this workspace')
      await dialog.getByRole('button', { name: 'Refresh status' }).click()
      await expect(dialog).toContainText('Another deletion operation owns part')
      await expect(
        dialog.getByRole('button', { name: 'Force delete…', exact: true }),
      ).toHaveCount(0)
      const retry = dialog.getByRole('button', {
        name: 'Retry delete',
        exact: true,
      })
      await expect(retry).toBeEnabled()
      await retry.focus()
      await expect(retry).toBeFocused()
      expect(
        await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth),
      ).toBe(true)
      await retry.press('Enter')
      await expect(dialog).toHaveCount(0)
    })
  }
}

for (const theme of ['light', 'dark']) {
  for (const [name, width, pane] of [
    ['phone', 390, 390],
    ['narrow', 1280, 420],
    ['wide', 1280, 1280],
  ] as const) {
    test(`${name} ${theme}: overlap review is explicit, safe and accessible`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 900 })
      await page.goto(`/?theme=${theme}&pane=${pane}&scenario=overlap`)
      if (name !== 'wide')
        await page
          .getByRole('button', { name: 'Back to conversations' })
          .click()
      await page.getByRole('button', { name: 'Request removal' }).click()
      const dialog = page.getByRole('dialog')
      await dialog.getByRole('button', { name: 'Delete', exact: true }).click()
      await expect(dialog.getByRole('alert')).toContainText(
        'Another deletion operation owns part',
      )
      await expect(
        dialog.getByRole('button', { name: 'Retry delete', exact: true }),
      ).toBeEnabled()
      await expect(
        dialog.getByRole('button', { name: 'Force delete…', exact: true }),
      ).toHaveCount(0)
      const review = dialog.getByRole('button', {
        name: 'Review existing deletion',
      })
      await review.focus()
      await expect(review).toBeFocused()
      expect(
        await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth),
      ).toBe(true)
      await review.press('Enter')
      await expect(
        dialog.getByRole('button', { name: 'Force delete…', exact: true }),
      ).toBeVisible()
      await expect(dialog).not.toContainText('Force delete this chat?')
      await dialog
        .getByRole('button', { name: 'Close', exact: true })
        .first()
        .click()
      await expect(dialog).toHaveCount(0)
    })
  }
}

for (const theme of ['light', 'dark']) {
  for (const [name, viewport, pane] of [
    ['phone', { width: 390, height: 844 }, 390],
    ['narrow', { width: 1280, height: 800 }, 420],
    ['wide', { width: 1280, height: 900 }, 1280],
  ] as const) {
    test(`${name} ${theme}: same dialog, safe focus, overflow and completion`, async ({
      page,
    }, testInfo) => {
      await page.setViewportSize(viewport)
      await page.goto(`/?theme=${theme}&pane=${pane}`)
      if (name !== 'wide')
        await page
          .getByRole('button', { name: 'Back to conversations' })
          .click()
      await page.getByRole('button', { name: 'Request removal' }).click()
      const dialog = page.getByRole('dialog')
      await expect(
        page.getByRole('button', { name: 'Cancel', exact: true }),
      ).toBeFocused()
      await expect(dialog).toContainText('its 1 subagent conversation')
      await page.getByRole('button', { name: 'Delete', exact: true }).click()
      await expect(dialog.getByRole('alert')).toContainText(
        'No conversations were deleted; data retained.',
      )
      await expect(dialog).not.toContainText(/pending/i)
      await expect(
        dialog.getByRole('list', { name: 'Deletion blockers' }),
      ).toContainText('browser::fetch')
      expect(
        await page.evaluate(
          () => document.documentElement.scrollWidth <= innerWidth,
        ),
      ).toBe(true)
      expect(
        await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth),
      ).toBe(true)
      await page.screenshot({ path: testInfo.outputPath('blocked.png') })
      await page
        .getByRole('button', { name: 'Force delete…', exact: true })
        .click()
      const back = page.getByRole('button', { name: 'Back', exact: true })
      await expect(back).toBeFocused()
      await expect(dialog).toContainText('Force delete this chat?')
      await expect(dialog).toContainText('External operations may continue')
      await page.screenshot({ path: testInfo.outputPath('confirm.png') })
      await back.press('Enter')
      await expect(dialog).not.toContainText('Force delete this chat?')
      await page
        .getByRole('button', { name: 'Force delete…', exact: true })
        .click()
      await back.press('Tab')
      const confirm = page.getByRole('button', {
        name: 'Force delete',
        exact: true,
      })
      await expect(confirm).toBeFocused()
      await confirm.press('Tab')
      expect(
        await dialog.evaluate((node) => node.contains(document.activeElement)),
      ).toBe(true)
      await confirm.click()
      await expect(dialog.getByRole('status')).toContainText('Force deleting…')
      await expect(
        page.getByRole('button', { name: 'Force deleting…', exact: true }),
      ).toBeDisabled()
      await expect(dialog).toHaveCount(0)
    })
  }
}

for (const scenario of ['partial', 'network']) {
  for (const [name, viewport, pane] of [
    ['phone', { width: 390, height: 844 }, 390],
    ['narrow', { width: 1280, height: 800 }, 420],
    ['wide', { width: 1280, height: 900 }, 1280],
  ] as const) {
    test(`${name}: ${scenario} is unconfirmed until explicit recovery`, async ({
      page,
    }, testInfo) => {
      await page.setViewportSize(viewport)
      await page.goto(`/?theme=light&pane=${pane}&scenario=${scenario}`)
      if (name !== 'wide')
        await page
          .getByRole('button', { name: 'Back to conversations' })
          .click()
      await page.getByRole('button', { name: 'Request removal' }).click()
      const dialog = page.getByRole('dialog')
      await dialog.getByRole('button', { name: 'Delete', exact: true }).click()
      await dialog
        .getByRole('button', { name: 'Force delete…', exact: true })
        .click()
      await dialog
        .getByRole('button', { name: 'Force delete', exact: true })
        .click()
      if (scenario === 'partial') {
        expect(await outcomeCounts(dialog)).toBe(
          'Confirmed deleted: 1. Not deleted: 0. Unconfirmed: 1.',
        )
        await dialog.locator('[data-deletion-outcomes] > summary').click()
        await expect(dialog).toContainText('Deleted sessions: grandchild1')
        await expect(dialog).toContainText(
          'Deletion outcome unconfirmed: child2',
        )
      } else
        await expect(dialog.getByRole('alert')).toContainText(
          'Completion is not confirmed. The backend may have continued.',
        )
      await expect(dialog).not.toContainText(/retained|pending/i)
      await expect(
        dialog.getByRole('button', { name: 'Force delete…', exact: true }),
      ).toHaveCount(0)
      expect(
        await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth),
      ).toBe(true)
      await page.screenshot({ path: testInfo.outputPath('unconfirmed.png') })
      await dialog
        .getByRole('button', {
          name: scenario === 'partial' ? 'Retry force delete' : 'Retry delete',
          exact: true,
        })
        .click()
      await expect(dialog).toHaveCount(0)
    })
  }
}

for (const scenario of ['legacy', 'rejected']) {
  for (const [name, viewport, pane] of [
    ['phone', { width: 390, height: 844 }, 390],
    ['narrow', { width: 1280, height: 800 }, 420],
    ['wide', { width: 1280, height: 900 }, 1280],
  ] as const) {
    test(`${name}: ${scenario} never claims data retention without proof`, async ({
      page,
    }) => {
      await page.setViewportSize(viewport)
      await page.goto(`/?theme=light&pane=${pane}&scenario=${scenario}`)
      if (name !== 'wide')
        await page
          .getByRole('button', { name: 'Back to conversations' })
          .click()
      await page.getByRole('button', { name: 'Request removal' }).click()
      const dialog = page.getByRole('dialog')
      await dialog.getByRole('button', { name: 'Delete', exact: true }).click()
      if (scenario === 'rejected') {
        await dialog
          .getByRole('button', { name: 'Force delete…', exact: true })
          .click()
        await dialog
          .getByRole('button', { name: 'Force delete', exact: true })
          .click()
        await expect(dialog).toContainText(
          'Force delete was rejected; nothing changed.',
        )
        await expect(dialog).not.toContainText(
          'The backend may have continued.',
        )
      } else {
        expect(await outcomeCounts(dialog)).toBe(
          'Confirmed deleted: 0. Not deleted: 0. Unconfirmed: 2.',
        )
      }
      await expect(dialog).not.toContainText(/retained|pending/i)
      expect(
        await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth),
      ).toBe(true)
    })
  }
}

for (const scenario of ['preplan', 'active']) {
  test(`${scenario}: outcome and blocker labels do not invent proof`, async ({
    page,
  }) => {
    await page.goto(`/?theme=light&pane=1280&scenario=${scenario}`)
    await page.getByRole('button', { name: 'Request removal' }).click()
    const dialog = page.getByRole('dialog')
    await dialog.getByRole('button', { name: 'Delete', exact: true }).click()
    if (scenario === 'preplan') {
      expect(await outcomeCounts(dialog)).toContain(
        'Not deleted: unknown. Unconfirmed: unknown.',
      )
      expect(await outcomeCounts(dialog)).not.toContain('Not deleted: 0')
      await expect(dialog).not.toContainText('data retained')
      await expect(
        dialog.getByRole('button', { name: 'Force delete…', exact: true }),
      ).toHaveCount(0)
    } else {
      await expect(dialog).toContainText('Active processing')
      await expect(dialog).not.toContainText('after timeout')
    }
  })
}

// Native ChatPanel and shared controls, with simulated frontend boundaries only.
// These examples do not exercise a live harness or perform any live deletion.
for (const theme of ['light', 'dark']) {
  for (const [name, viewport, pane] of [
    ['phone', { width: 390, height: 844 }, 390],
    ['narrow', { width: 1280, height: 800 }, 420],
    ['wide', { width: 1280, height: 900 }, 1280],
  ] as const) {
    for (const count of [1, 24, 1000]) {
      test(`${name} ${theme}: ${count} blockers are bounded, searchable and progressively disclosed`, async ({
        page,
      }, testInfo) => {
        await page.setViewportSize(viewport)
        await page.goto(
          `/?theme=${theme}&pane=${pane}&blockers=${count}&kinds=mixed`,
        )
        if (name !== 'wide')
          await page
            .getByRole('button', { name: 'Back to conversations' })
            .click()
        await page.getByRole('button', { name: 'Request removal' }).click()
        const dialog = page.getByRole('dialog')
        await dialog
          .getByRole('button', { name: 'Delete', exact: true })
          .click()
        const rows = dialog.locator('[data-deletion-blocker]')
        await expect(rows).toHaveCount(Math.min(count, 24))
        await expect(dialog.locator('details[open]')).toHaveCount(0)
        await expect(dialog).toContainText(
          `${Math.ceil(count / 12)} affected chat`,
        )
        expect(
          await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth),
        ).toBe(true)
        expect(
          await page.evaluate(
            () => document.documentElement.scrollWidth <= innerWidth,
          ),
        ).toBe(true)
        const footer = dialog.locator('[data-chat-delete-actions]')
        const footerBox = await footer.boundingBox()
        expect(footerBox).not.toBeNull()
        expect((footerBox?.y ?? -1) >= 0).toBe(true)
        expect(
          (footerBox?.y ?? 0) + (footerBox?.height ?? 0),
        ).toBeLessThanOrEqual(viewport.height)
        await page.screenshot({ path: testInfo.outputPath('many-blocked.png') })
        const search = dialog.getByRole('searchbox', {
          name: 'Filter deletion blockers',
        })
        await search.fill(`call-${count - 1}-`)
        await expect(rows).toHaveCount(1)
        await expect(dialog).toContainText(`1 of ${count} blockers match`)
        await rows.first().locator('summary').focus()
        await page.keyboard.press('Enter')
        const detail = rows.first().locator('[data-deletion-blocker-detail]')
        await expect(detail).toContainText(`call-${count - 1}-`)
        await expect(detail).toContainText('1970-01-01T00:00:00.001Z')
        await expect(detail.locator('dd').nth(1)).toContainText('s'.repeat(100))
        const summaryBox = await rows.first().locator('summary').boundingBox()
        expect(summaryBox?.height).toBeGreaterThanOrEqual(48)
        const inputBox = await search.boundingBox()
        expect(inputBox?.height).toBeGreaterThanOrEqual(48)
        expect(
          await search.evaluate((node) => getComputedStyle(node).fontSize),
        ).toBe('16px')
        expect(
          await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth),
        ).toBe(true)
        await page.screenshot({ path: testInfo.outputPath('many-details.png') })
        await search.fill('no-such-operation')
        await expect(dialog).toContainText('No matching blockers')
        await expect(rows).toHaveCount(0)
        await expect(
          dialog.getByRole('button', { name: 'Force delete…', exact: true }),
        ).toBeEnabled()
        await search.press('Escape')
        await expect(search).toHaveValue('')
        await expect(search).toBeFocused()
        await expect(dialog).toHaveCount(1)
        await expect(rows).toHaveCount(Math.min(count, 24))
        if (count === 1000) {
          await dialog
            .getByRole('navigation', { name: 'Blocker pages' })
            .getByRole('button', { name: 'Next' })
            .click()
          await expect(dialog).toContainText('Showing 25–48 of 1000 blockers')
          await expect(rows).toHaveCount(24)
        }
        await dialog
          .getByRole('button', { name: 'Force delete…', exact: true })
          .click()
        await expect(
          dialog.getByRole('button', { name: 'Back', exact: true }),
        ).toBeFocused()
        await expect(dialog).toContainText(
          'External operations may continue; previous side effects are not undone.',
        )
        await expect(page.getByRole('dialog')).toHaveCount(1)
        await dialog.getByRole('button', { name: 'Back', exact: true }).click()
        await expect(rows).toHaveCount(Math.min(count, 24))
        await dialog
          .getByRole('button', { name: 'Close', exact: true })
          .first()
          .click()
        await expect(dialog).toHaveCount(0)
      })
    }
  }
}

// Focused finishing checks; the existing scale/theme matrix remains unchanged.
for (const theme of ['light', 'dark']) {
  for (const [name, viewport, pane] of [
    ['phone', { width: 390, height: 844 }, 390],
    ['narrow', { width: 1280, height: 800 }, 420],
  ] as const) {
    test(`${name} ${theme}: finishing a11y title, outcome scrolling and keyboard boundaries`, async ({
      page,
    }) => {
      await page.setViewportSize(viewport)
      for (const scenario of ['a11y', 'longerror']) {
        await page.goto(`/?theme=${theme}&pane=${pane}&scenario=${scenario}`)
        await page
          .getByRole('button', { name: 'Back to conversations' })
          .click()
        await page.getByRole('button', { name: 'Request removal' }).click()
        const dialog = page.getByRole('dialog')
        const description = dialog.locator('[data-chat-delete-description]')
        const flowingText = async (
          locator: typeof description,
          phrases: string[],
        ) => {
          expect(
            await locator.evaluate((node, expected) => {
              const style = getComputedStyle(node)
              const body = node.closest('[data-chat-delete-body]')
              if (!body) throw new Error('Missing dialog body')
              return {
                maxHeight: style.maxHeight,
                overflowY: style.overflowY,
                innerClip: node.scrollHeight > node.clientHeight + 1,
                phrasesVisibleByBodyScroll: expected.every((phrase) => {
                  const walker = document.createTreeWalker(
                    node,
                    NodeFilter.SHOW_TEXT,
                  )
                  let text = walker.nextNode()
                  while (text && !text.textContent?.includes(phrase))
                    text = walker.nextNode()
                  if (!text) return false
                  const range = document.createRange()
                  const start = text.textContent?.indexOf(phrase) ?? -1
                  range.setStart(text, start)
                  range.setEnd(text, start + phrase.length)
                  const rect = range.getBoundingClientRect()
                  const bodyRect = body.getBoundingClientRect()
                  body.scrollTop += Math.max(
                    0,
                    rect.bottom - bodyRect.bottom + 8,
                  )
                  const visible = range.getBoundingClientRect()
                  return (
                    visible.top >= bodyRect.top &&
                    visible.bottom <= bodyRect.bottom
                  )
                }),
              }
            }, phrases),
          ).toEqual({
            maxHeight: 'none',
            overflowY: 'visible',
            innerClip: false,
            phrasesVisibleByBodyScroll: true,
          })
        }
        await flowingText(description, [
          'This cannot be undone.',
          'Running work will be stopped first.',
          'The parent will be notified, not stopped or deleted.',
        ])
        await dialog
          .getByRole('button', { name: 'Stop and delete', exact: true })
          .click()
        if (scenario === 'longerror') {
          await flowingText(dialog.locator('[data-chat-delete-error]'), [
            'End of simulated failure.',
          ])
          await expect(
            dialog.getByRole('button', { name: 'Force delete…', exact: true }),
          ).toHaveCount(0)
        } else {
          const summary = dialog.locator('[data-deletion-outcomes] > summary')
          await summary.focus()
          await page.keyboard.press('Enter')
          const list = dialog.getByRole('list', {
            name: 'Reported deletion outcomes',
          })
          await expect(list.locator('li')).toHaveCount(24)
          await expect(list).toHaveAttribute('tabindex', '0')
          await page.keyboard.press('Tab')
          await expect(list).toBeFocused()
          expect(
            await list.evaluate((node) => ({
              overflowing: node.scrollHeight > node.clientHeight,
              outlineWidth: getComputedStyle(node).outlineWidth,
              outlineStyle: getComputedStyle(node).outlineStyle,
            })),
          ).toEqual({
            overflowing: true,
            outlineWidth: '2px',
            outlineStyle: 'solid',
          })
          await page.keyboard.press('PageDown')
          await expect
            .poll(() => list.evaluate((node) => node.scrollTop))
            .toBeGreaterThan(0)
          await page.keyboard.press('End')
          await expect
            .poll(() =>
              list.evaluate(
                (node) =>
                  node.scrollTop + node.clientHeight >= node.scrollHeight - 1,
              ),
            )
            .toBe(true)
          await expect(list).toBeFocused()
          await page.keyboard.press('Tab')
          const outcomeNav = dialog.getByRole('navigation', {
            name: 'Outcome pages',
          })
          await expect(
            outcomeNav.getByRole('button', { name: 'Previous' }),
          ).toBeFocused()
          for (const label of ['Outcome pages', 'Blocker pages']) {
            const nav = dialog.getByRole('navigation', { name: label })
            const previous = nav.getByRole('button', { name: 'Previous' })
            const next = nav.getByRole('button', { name: 'Next' })
            await previous.focus()
            await expect(previous).toHaveAttribute('aria-disabled', 'true')
            await previous.press('Enter')
            await expect(previous).toBeFocused()
            await expect(nav).toContainText('Page 1 of 2')
            await next.focus()
            await next.press('Enter')
            await expect(next).toHaveAttribute('aria-disabled', 'true')
            await expect(next).toBeFocused()
            await next.press('Space')
            await expect(next).toBeFocused()
            await expect(nav).toContainText('Page 2 of 2')
            await previous.focus()
            await previous.press('Space')
            await previous.press('Enter')
            await expect(previous).toBeFocused()
            await expect(nav).toContainText('Page 1 of 2')
            await expect(nav.locator('[aria-live]')).toHaveCount(0)
          }
          await expect(
            dialog.locator('[data-deletion-outcomes] [aria-live="polite"]'),
          ).toHaveCount(1)
          await expect(
            dialog.locator('[data-deletion-blockers] [aria-live="polite"]'),
          ).toHaveCount(1)
          await dialog
            .getByRole('button', { name: 'Force delete…', exact: true })
            .click()
          await expect(dialog).toContainText(
            'External operations may continue; previous side effects are not undone.',
          )
          await expect(dialog).toHaveAttribute('aria-describedby', /\S+ \S+/)
          await flowingText(description, [
            'This cannot be undone.',
            'Running work will be cancelled on a best-effort basis.',
            'The parent will be notified, not stopped or deleted.',
          ])
        }
        const footer = dialog.locator('[data-chat-delete-actions]')
        const box = await footer.boundingBox()
        expect(box).not.toBeNull()
        expect(box?.y).toBeGreaterThanOrEqual(0)
        expect((box?.y ?? 0) + (box?.height ?? 0)).toBeLessThanOrEqual(
          viewport.height,
        )
        expect(
          await footer
            .getByRole('button')
            .evaluateAll((nodes) =>
              nodes.every((node) => node.getBoundingClientRect().height >= 48),
            ),
        ).toBe(true)
        expect(
          await dialog.evaluate((node) => node.scrollWidth <= node.clientWidth),
        ).toBe(true)
        expect(
          await page.evaluate(
            () => document.documentElement.scrollWidth <= innerWidth,
          ),
        ).toBe(true)
      }
    })
  }
}
