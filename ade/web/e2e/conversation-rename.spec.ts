import { expect, expectPassingResult, openSession, test } from './harness-stack'

test.describe('desktop rename focus', () => {
  test.use({ scenario: 'streamed-text' })

  test('every exit from the rename dialog restores row focus, and Tab stays inside it', async ({
    page,
    stack,
  }) => {
    const completed = stack.waitForTurnCompleted()
    await stack.trigger()
    expect(await completed).toMatchObject({ status: 'completed' })

    await page.setViewportSize({ width: 1280, height: 800 })
    await openSession(page, stack)

    /* Renaming is a dialog (the row is too narrow to edit a whole title):
       F2, a double-click or the row's ⋯ menu opens it, Save / Enter keep
       the new name, Cancel / Escape drop it, and every exit hands focus
       back to the row. */
    let title = stack.ready.session.title
    const input = page.getByRole('textbox', { name: 'Chat name', exact: true })
    const row = (name: string) =>
      page.getByRole('button', { name: `open ${name}`, exact: true })
    for (const action of ['Save', 'Cancel', 'Enter', 'Escape'] as const) {
      await row(title).focus()
      await row(title).press('F2')
      await expect(input).toBeFocused()
      const draft = `${stack.ready.session.title} ${action}`
      await input.fill(draft)
      if (action === 'Save' || action === 'Cancel') {
        await page
          .getByRole('dialog')
          .getByRole('button', { name: action, exact: true })
          .click()
      } else {
        await input.press(action)
      }
      if (action === 'Save' || action === 'Enter') title = draft
      await expect(input).toHaveCount(0)
      await expect(row(title)).toBeFocused()
    }

    // The ⋯ menu reaches the same dialog, and Tab stays inside it (modal):
    // the field, Cancel, Save, then the dialog's close, never the page.
    await row(title).hover()
    await page
      .getByRole('button', { name: `more actions for ${title}`, exact: true })
      .click()
    await page.getByRole('menuitem', { name: 'Rename' }).click()
    await expect(input).toBeFocused()
    for (let step = 0; step < 4; step++) {
      await page.keyboard.press('Tab')
      await expect(
        page.getByRole('dialog').locator(':focus'),
        `Tab ${step + 1} left the rename dialog`,
      ).toHaveCount(1)
    }
    await page.keyboard.press('Escape')
    await expect(input).toHaveCount(0)
    await expect(row(title)).toBeFocused()

    expectPassingResult(await stack.finish())
  })
})

test.describe('touch rows', () => {
  test.use({ scenario: 'streamed-text', hasTouch: true })

  test('drop the rename pencil and keep the row menu reachable', async ({
    page,
    stack,
  }) => {
    const completed = stack.waitForTurnCompleted()
    await stack.trigger()
    expect(await completed).toMatchObject({ status: 'completed' })

    await page.setViewportSize({ width: 1280, height: 800 })
    await openSession(page, stack)
    await page.setViewportSize({ width: 390, height: 844 })
    await page.getByRole('button', { name: 'back to conversations' }).tap()

    const title = stack.ready.session.title
    const row = page.getByRole('button', { name: `open ${title}`, exact: true })
    await expect(row).toBeVisible()

    /* Renaming is a desktop affordance — it pairs with double-click and F2.
       A coarse pointer never sees the pencil, which leaves touch rows the
       single trailing action the tree recipe reserves room for. */
    await expect(
      row.getByRole('button', { name: `rename ${title}`, exact: true }),
    ).toBeHidden()

    /* The row's one trailing action — the ⋯ menu that holds Rename and
       Delete — survives, and survives finger-sized: a coarse pointer gets
       48px whatever the row measures. */
    const more = row.getByRole('button', {
      name: `more actions for ${title}`,
      exact: true,
    })
    await expect(more).toBeVisible()
    await expect(more).toBeEnabled()
    const target = await more.boundingBox()
    expect(target?.width ?? 0).toBeGreaterThanOrEqual(44)
    expect(target?.height ?? 0).toBeGreaterThanOrEqual(44)
    await more.tap()
    await expect(page.getByRole('menuitem', { name: /^Delete/ })).toBeVisible()
    await page.keyboard.press('Escape')

    expectPassingResult(await stack.finish())
  })
})
