import { expect, expectPassingResult, openSession, test } from './harness-stack'

for (const mobile of [false, true]) {
  test.describe(mobile ? 'mobile rename focus' : 'desktop rename focus', () => {
    test.use({ scenario: 'streamed-text', hasTouch: mobile })

    test('explicit exits restore row focus while Tab preserves its destination', async ({
      page,
      stack,
    }) => {
      const completed = stack.waitForTurnCompleted()
      await stack.trigger()
      expect(await completed).toMatchObject({ status: 'completed' })

      await page.setViewportSize({ width: 1280, height: 800 })
      await openSession(page, stack)
      if (mobile) {
        await page.setViewportSize({ width: 390, height: 844 })
        await page.getByRole('button', { name: 'back to conversations' }).tap()
      }

      let title = stack.ready.session.title
      const input = page.getByRole('textbox', {
        name: 'conversation title',
        exact: true,
      })
      for (const action of ['Save', 'Cancel', 'Enter', 'Escape'] as const) {
        const row = page.getByRole('button', {
          name: `open ${title}`,
          exact: true,
        })
        const rename = row.getByRole('button', {
          name: `rename ${title}`,
          exact: true,
        })
        if (mobile) {
          await rename.tap()
        } else {
          await row.focus()
          await rename.press('Enter')
        }
        await expect(input).toBeFocused()
        const draft = `${stack.ready.session.title} ${action}`
        await input.fill(draft)
        if (action === 'Save' || action === 'Cancel') {
          const button = page.getByRole('button', {
            name:
              action === 'Save' ? 'Save conversation title' : 'Cancel rename',
            exact: true,
          })
          if (mobile) await button.tap()
          else await button.click()
        } else {
          await input.press(action)
        }
        if (action === 'Save' || action === 'Enter') title = draft
        await expect(input).toHaveCount(0)
        await expect(
          page.getByRole('button', { name: `open ${title}`, exact: true }),
        ).toBeFocused()
        if (mobile)
          await expect(page.getByLabel('search conversations')).toBeVisible()
      }

      // A prior explicit exit must not leave a stale restore-focus request.
      const row = page.getByRole('button', {
        name: `open ${title}`,
        exact: true,
      })
      await row.press('F2')
      await expect(input).toBeFocused()
      const tabTitle = `${stack.ready.session.title} Tab`
      await input.fill(tabTitle)
      await input.press('Tab')
      await expect(
        page.getByRole('button', { name: 'Save conversation title' }),
      ).toBeFocused()
      await page.keyboard.press('Tab')
      const cancel = page.getByRole('button', {
        name: 'Cancel rename',
        exact: true,
      })
      await expect(cancel).toBeFocused()
      await cancel.press('Tab')
      await expect(input).toHaveCount(0)
      await expect(
        page.getByRole('button', { name: `open ${tabTitle}`, exact: true }),
      ).not.toBeFocused()
      await expect(page.locator(':focus')).not.toHaveJSProperty(
        'tagName',
        'BODY',
      )
      await expect(page.locator(':focus')).toHaveCount(1)

      expectPassingResult(await stack.finish())
    })
  })
}
