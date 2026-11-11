import { expect, type Page, test } from '@playwright/test'

/**
 * What the transcript window has to keep true in a real layout. jsdom has none,
 * so a green unit suite says nothing about any of this. The page is the real
 * `MessageList` over a generated transcript, with no engine behind it.
 */

declare global {
  interface Window {
    __transcript?: {
      append: (turns: number) => void
      prepend: (turns: number) => void
      prependCalls: (id: string, calls: number) => void
      startBatch: (id: string, calls: number) => void
      settleBatch: (id: string) => void
      stream: (text: string) => void
      finishStream: () => void
      focus: (id: string | null) => void
      holdForApproval: (turn: number) => void
      setWorking: (working: boolean) => void
      holdCall: (id: string) => void
    }
    __sidebar?: { touch: (id: string) => void }
    __hiddenTab?: boolean
  }
}

const ROW = '.chat-message-stack > [data-message-row]'
const rowSelector = (key: string) => `${ROW}[data-message-row="${key}"]`
/** Viewport (900px) plus overscan: bounded however long the chat gets. */
const MAX_MOUNTED_ROWS = 60

const settle = (page: Page) =>
  page.evaluate(
    () =>
      new Promise<void>((resolve) =>
        requestAnimationFrame(() =>
          requestAnimationFrame(() => setTimeout(resolve, 150)),
        ),
      ),
  )

const open = async (page: Page, query: string) => {
  await page.goto(`/transcript-stress.html?${query}`)
  await expect(page.locator('[data-transcript-opening]')).toHaveCount(0)
  await settle(page)
}

const metrics = (page: Page) =>
  page.evaluate((row) => {
    const list = document.querySelector<HTMLElement>('[data-message-list]')
    if (!list) throw new Error('no list')
    return {
      rows: list.querySelectorAll(row).length,
      nodes: list.querySelectorAll('*').length,
      spacers: list.querySelectorAll('[data-transcript-spacer]').length,
      scrollTop: list.scrollTop,
      scrollHeight: list.scrollHeight,
      clientHeight: list.clientHeight,
      tail: list.dataset.tailScroll,
    }
  }, ROW)

/** The tail glide takes a moment to land after the content last grew. */
const expectAtTail = (page: Page) =>
  expect
    .poll(async () => {
      const m = await metrics(page)
      return m.scrollHeight - m.clientHeight - m.scrollTop
    })
    .toBeLessThan(2)

/** Scroll as a reader does: the wheel event is what pauses the tail. */
const scrollTo = async (page: Page, top: number) => {
  await page.evaluate((y) => {
    const list = document.querySelector<HTMLElement>('[data-message-list]')
    list?.dispatchEvent(new WheelEvent('wheel', { deltaY: -1, bubbles: true }))
    if (list) list.scrollTop = y
  }, top)
  await settle(page)
}

/** Bring a row back the way a reader does, from the far end, never via a jump. */
const scrollUntilMounted = async (page: Page, key: string) => {
  const { scrollHeight } = await metrics(page)
  for (let top = 0; top < scrollHeight; top += 500) {
    await scrollTo(page, top)
    if ((await page.locator(rowSelector(key)).count()) > 0) return
  }
  throw new Error(`row ${key} never mounted`)
}

/** The first row in view and where it sits, marking the node. Rows run taller
 * than the viewport, so it can start above the top edge. */
const anchor = (page: Page) =>
  page.evaluate((row) => {
    const list = document.querySelector<HTMLElement>('[data-message-list]')
    if (!list) throw new Error('no list')
    const box = list.getBoundingClientRect()
    const node = [...list.querySelectorAll<HTMLElement>(row)].find((el) => {
      const rect = el.getBoundingClientRect()
      return rect.bottom > box.top && rect.top < box.bottom
    })
    if (!node) throw new Error('no row in view')
    ;(node as HTMLElement & { __mark?: boolean }).__mark = true
    return {
      key: node.dataset.messageRow as string,
      top: node.getBoundingClientRect().top,
    }
  }, ROW)

const rowState = (page: Page, key: string) =>
  page.evaluate((id) => {
    const node = document.querySelector<HTMLElement & { __mark?: boolean }>(
      `.chat-message-stack > [data-message-row="${id}"]`,
    )
    return node
      ? { top: node.getBoundingClientRect().top, marked: node.__mark === true }
      : null
  }, key)

/** Fraction of the viewport covered by rows, the worst frame while `run` goes. */
const worstCoverage = async (page: Page, run: () => Promise<void>) => {
  await page.evaluate((row) => {
    const list = document.querySelector<HTMLElement>('[data-message-list]')
    if (!list) throw new Error('no list')
    const state = { min: 1, stop: false }
    ;(window as unknown as { __coverage: typeof state }).__coverage = state
    const tick = () => {
      const box = list.getBoundingClientRect()
      let covered = 0
      for (const el of list.querySelectorAll(row)) {
        const rect = el.getBoundingClientRect()
        covered += Math.max(
          0,
          Math.min(rect.bottom, box.bottom) - Math.max(rect.top, box.top),
        )
      }
      state.min = Math.min(state.min, covered / box.height)
      if (!state.stop) requestAnimationFrame(tick)
    }
    requestAnimationFrame(tick)
  }, ROW)
  await run()
  return page.evaluate(() => {
    const state = (
      window as unknown as { __coverage: { min: number; stop: boolean } }
    ).__coverage
    state.stop = true
    return state.min
  })
}

test.describe('transcript window', () => {
  test('a short chat mounts every row and needs no spacer', async ({
    page,
  }) => {
    await open(page, 'turns=2')
    const { rows, spacers } = await metrics(page)
    expect(rows).toBeGreaterThan(0)
    expect(spacers).toBe(0)
  })

  test('opens at the tail with only the rows around it', async ({ page }) => {
    await open(page, 'turns=300')
    const m = await metrics(page)
    expect(m.rows).toBeLessThan(MAX_MOUNTED_ROWS)
    expect(m.spacers).toBeGreaterThan(0)
    expect(m.scrollHeight - m.clientHeight - m.scrollTop).toBeLessThan(2)
    expect(m.tail).toBe('following')
    await expect(page.locator(rowSelector('reply-299'))).toBeInViewport()
  })

  test('a session left open stays bounded as rows arrive', async ({ page }) => {
    await open(page, 'turns=30')
    const before = await metrics(page)
    for (let round = 0; round < 40; round++) {
      await page.evaluate(() => window.__transcript?.append(5))
      await settle(page)
    }
    const after = await metrics(page)
    expect(after.rows).toBeLessThan(MAX_MOUNTED_ROWS)
    expect(after.nodes).toBeLessThan(before.nodes * 3)
    // Still on the tail: the reader never had to chase it.
    expect(after.tail).toBe('following')
    await expectAtTail(page)
    await expect(page.locator(rowSelector('reply-229'))).toBeInViewport()
  })

  test('batches that ran live leave the DOM once settled', async ({ page }) => {
    await open(page, 'turns=30')
    for (let round = 0; round < 12; round++) {
      const id = `live${round}`
      await page.evaluate((batch) => {
        window.__transcript?.startBatch(batch, 6)
      }, id)
      await settle(page)
      await expect(page.locator(rowSelector(`fc_${id}-0`))).toHaveCount(1)
      await page.evaluate((batch) => {
        window.__transcript?.settleBatch(batch)
        window.__transcript?.append(3)
      }, id)
      await settle(page)
    }
    expect((await metrics(page)).rows).toBeLessThan(MAX_MOUNTED_ROWS)
    await expect(page.locator(rowSelector('fc_live0-0'))).toHaveCount(0)
  })

  test('streaming keeps following the tail', async ({ page }) => {
    await open(page, 'turns=100')
    for (let token = 0; token < 30; token++) {
      await page.evaluate(() =>
        window.__transcript?.stream(
          'A streaming sentence of the reply. '.repeat(8),
        ),
      )
      await settle(page)
    }
    expect((await metrics(page)).tail).toBe('following')
    await expectAtTail(page)
  })

  test('a tail follow that stops under a pixel still lands', async ({
    page,
  }) => {
    await open(page, 'turns=100')
    await page.evaluate(() => window.__transcript?.stream('A short token.'))
    await expectAtTail(page)
  })

  test('the viewport stays covered scrolling both ways', async ({ page }) => {
    await open(page, 'turns=200')
    await page.hover('[data-message-list]')
    const wheel = (delta: number) => async () => {
      for (let step = 0; step < 40; step++) {
        await page.mouse.wheel(0, delta)
        await page.evaluate(
          () =>
            new Promise((resolve) =>
              requestAnimationFrame(() => resolve(null)),
            ),
        )
      }
    }
    expect(await worstCoverage(page, wheel(-400))).toBeGreaterThan(0.5)
    expect(await worstCoverage(page, wheel(400))).toBeGreaterThan(0.5)
  })

  test('reaches the first row and stays bounded', async ({ page }) => {
    await open(page, 'turns=200')
    await scrollTo(page, 0)
    await expect(page.locator(rowSelector('user-0'))).toBeInViewport()
    expect((await metrics(page)).rows).toBeLessThan(MAX_MOUNTED_ROWS)
  })

  test('the wheel never jumps while rows above measure in', async ({
    page,
  }) => {
    await open(page, 'turns=200')
    await page.hover('[data-message-list]')
    // Rows above the tail have only estimates: every step proves the correction
    // does not reach the reader. A row missing afterward is itself a jump.
    for (let step = 0; step < 40; step++) {
      const before = await anchor(page)
      await page.mouse.wheel(0, -180)
      await settle(page)
      const after = await rowState(page, before.key)
      expect(after, `${before.key} left at step ${step}`).not.toBeNull()
      expect(Math.abs((after?.top ?? 0) - before.top - 180)).toBeLessThan(3)
    }
  })

  test('the wheel never jumps scrolling down from the top', async ({
    page,
  }) => {
    await open(page, 'turns=100')
    await scrollTo(page, 0)
    await page.hover('[data-message-list]')
    // The first spacer appears here: it has to stand in for exactly the rows it
    // replaces, its own gap included.
    for (let step = 0; step < 30; step++) {
      const before = await anchor(page)
      await page.mouse.wheel(0, 180)
      await settle(page)
      const after = await rowState(page, before.key)
      expect(after, `${before.key} left at step ${step}`).not.toBeNull()
      expect(Math.abs((after?.top ?? 0) - before.top + 180)).toBeLessThan(3)
    }
  })

  test('a burst of rows keeps the viewport covered during the glide', async ({
    page,
  }) => {
    await open(page, 'turns=60')
    const worst = await worstCoverage(page, async () => {
      await page.evaluate(() => window.__transcript?.append(15))
      await page.waitForTimeout(1200)
    })
    expect(worst).toBeGreaterThan(0.5)
    await expectAtTail(page)
  })

  test('jumping back to the tail over measured rows keeps the view covered', async ({
    page,
  }) => {
    // Short rows, all measured on the way up: on the way back nothing resizes,
    // so nothing but the scroll itself can move the window along with the glide.
    await open(page, 'turns=300&uniform=1')
    await page.hover('[data-message-list]')
    for (let step = 0; step < 5; step++) {
      await page.mouse.wheel(0, -300)
      await settle(page)
    }
    const worst = await worstCoverage(page, async () => {
      await page.getByRole('button', { name: 'jump to latest message' }).click()
      await page.waitForTimeout(800)
    })
    expect(worst).toBeGreaterThan(0.5)
    await expectAtTail(page)
  })

  test('a scrollbar drag lands on the row its position promised', async ({
    page,
  }) => {
    // Every row is far over the estimate and none above the tail is measured,
    // so a row's estimated place is known exactly, and corrections are large.
    await open(page, 'turns=200&uniform=46')
    for (const y of [32000, 16000, 6400]) {
      await scrollTo(page, y)
      const expected = await page.evaluate(
        (key) => {
          const list = document.querySelector<HTMLElement>(
            '[data-message-list]',
          )
          const box = list?.getBoundingClientRect()
          const rect = document
            .querySelector(`.chat-message-stack > [data-message-row="${key}"]`)
            ?.getBoundingClientRect()
          return (
            !!box && !!rect && rect.bottom > box.top && rect.top < box.bottom
          )
        },
        `row-${Math.floor(y / 320)}`,
      )
      expect(expected, `row at ${y} left the view`).toBe(true)
    }
  })

  test('a far landing centers a short row between tall ones', async ({
    page,
  }) => {
    // Rows mounting between the viewport top and the target are in view, so no
    // correction covers them: they push it down unless the landing settles.
    await open(page, 'turns=200&uniform=46')
    await page.evaluate(() => window.__transcript?.focus('row-40'))
    await settle(page)
    const offCenter = await page.evaluate(() => {
      const list = document.querySelector<HTMLElement>('[data-message-list]')
      const box = list?.getBoundingClientRect()
      const rect = document
        .querySelector('.chat-message-stack > [data-message-row="row-40"]')
        ?.getBoundingClientRect()
      if (!box || !rect) return Number.POSITIVE_INFINITY
      return Math.abs(rect.top + rect.height / 2 - (box.top + box.height / 2))
    })
    expect(offCenter).toBeLessThan(100)
  })

  test('a dragged scrollbar keeps the row it landed on', async ({ page }) => {
    await open(page, 'turns=200')
    const { scrollHeight } = await metrics(page)
    for (const fraction of [0.7, 0.4, 0.15]) {
      await page.evaluate((y) => {
        const list = document.querySelector<HTMLElement>('[data-message-list]')
        list?.dispatchEvent(
          new WheelEvent('wheel', { deltaY: -1, bubbles: true }),
        )
        if (list) list.scrollTop = y
      }, scrollHeight * fraction)
      await page.evaluate(
        () =>
          new Promise((resolve) => requestAnimationFrame(() => resolve(null))),
      )
      const landed = await anchor(page).catch(() => null)
      await settle(page)
      if (!landed) continue
      const after = await rowState(page, landed.key)
      expect(after).not.toBeNull()
      const box = await metrics(page)
      expect(after?.top ?? -1).toBeGreaterThan(-box.clientHeight)
      expect(after?.top ?? 1e9).toBeLessThan(box.clientHeight)
    }
  })

  test('landing older history keeps a reader mid-list on the same rows', async ({
    page,
  }) => {
    await open(page, 'turns=60&more=1')
    const { scrollHeight } = await metrics(page)
    await scrollTo(page, Math.floor(scrollHeight / 2))
    const before = await anchor(page)
    await page.evaluate(() => window.__transcript?.prepend(20))
    await settle(page)
    const after = await rowState(page, before.key)
    expect(Math.abs((after?.top ?? 0) - before.top)).toBeLessThan(2)
    // The same node: a prepend commit must not unmount the rows on screen.
    expect(after?.marked).toBe(true)
    expect((await metrics(page)).rows).toBeLessThan(MAX_MOUNTED_ROWS)
  })

  test('landing older history keeps a reader near the top too', async ({
    page,
  }) => {
    await open(page, 'turns=60&more=1')
    await scrollTo(page, 200)
    const before = await anchor(page)
    await page.evaluate(() => window.__transcript?.prepend(20))
    await settle(page)
    const after = await rowState(page, before.key)
    expect(Math.abs((after?.top ?? 0) - before.top)).toBeLessThan(2)
  })

  test('older history landing on a short chat is corrected once', async ({
    page,
  }) => {
    await open(page, 'turns=2&more=1')
    expect((await metrics(page)).spacers).toBe(0)
    await scrollTo(page, 300)
    const before = await anchor(page)
    await page.evaluate(() => window.__transcript?.prepend(1))
    await settle(page)
    const after = await rowState(page, before.key)
    expect(Math.abs((after?.top ?? 0) - before.top)).toBeLessThan(2)
  })

  test('a page merging into the head batch keeps it mounted', async ({
    page,
  }) => {
    await open(page, 'turns=60&more=1&headbatch=1')
    await scrollTo(page, 0)
    const head = await anchor(page)
    await page.evaluate(() => window.__transcript?.prependCalls('older', 3))
    await settle(page)
    const after = await rowState(page, head.key)
    // The batch took a new first item but is the same row under the same key.
    expect(after?.marked).toBe(true)
  })

  test('a row the reader opened stays mounted and open', async ({ page }) => {
    await open(page, 'turns=100')
    const keys = await page.evaluate((row) => {
      const of = (selector: string) =>
        [...document.querySelectorAll<HTMLElement>(row)]
          .filter((el) => el.querySelector(selector))
          .map((el) => el.dataset.messageRow as string)
      return of('[data-function-call-group]')
    }, ROW)
    const collapsible = await page.evaluate((row) => {
      const found = [...document.querySelectorAll<HTMLElement>(row)].filter(
        (el) =>
          el.querySelector(
            '[data-function-call-group][data-can-collapse="true"]',
          ),
      )
      return found[found.length - 1]?.dataset.messageRow as string
    }, ROW)
    expect(collapsible).toBeTruthy()
    const untouched = keys.find((key) => key !== collapsible) as string
    const group = `${rowSelector(collapsible)} [data-function-call-group]`
    await page.locator(`${group} .chat-tool-group__head`).click()
    await expect(page.locator(group)).toHaveAttribute('data-expanded', 'true')
    await scrollTo(page, 0)
    // Proves the pin, not just the state: its untouched neighbor is gone.
    await expect(page.locator(rowSelector(untouched))).toHaveCount(0)
    await expect(page.locator(group)).toHaveAttribute('data-expanded', 'true')
  })

  test('a batch stays open between its calls and folds once the agent moves on', async ({
    page,
  }) => {
    await open(page, 'turns=100')
    await page.evaluate(() => {
      window.__transcript?.setWorking(true)
      window.__transcript?.startBatch('live', 4)
    })
    await settle(page)
    const group = `${rowSelector('fc_live-0')} [data-function-call-group]`
    await expect(page.locator(group)).toHaveAttribute('data-expanded', 'true')
    // Between one call finishing and the next starting, none is running.
    await page.evaluate(() => window.__transcript?.settleBatch('live'))
    await settle(page)
    await expect(page.locator(group)).toHaveAttribute('data-expanded', 'true')
    await page.evaluate(() => window.__transcript?.startBatch('more', 2))
    await settle(page)
    await expect(page.locator(group)).toHaveAttribute('data-expanded', 'true')
    await expect(page.locator(group)).toHaveAttribute(
      'data-function-call-count',
      '6',
    )
    await page.evaluate(() => {
      window.__transcript?.settleBatch('more')
      window.__transcript?.stream('That covers those calls.')
    })
    await expect(page.locator(group)).toHaveAttribute('data-expanded', 'false')
    expect((await metrics(page)).tail).toBe('following')
    await expectAtTail(page)
  })

  test('a batch folds when the turn ends', async ({ page }) => {
    await open(page, 'turns=100')
    await page.evaluate(() => {
      window.__transcript?.setWorking(true)
      window.__transcript?.startBatch('live', 4)
    })
    await settle(page)
    const group = `${rowSelector('fc_live-0')} [data-function-call-group]`
    await page.evaluate(() => window.__transcript?.settleBatch('live'))
    await settle(page)
    await expect(page.locator(group)).toHaveAttribute('data-expanded', 'true')
    await page.evaluate(() => window.__transcript?.setWorking(false))
    await expect(page.locator(group)).toHaveAttribute('data-expanded', 'false')
  })

  test('a batch the reader chose stays as they left it', async ({ page }) => {
    await open(page, 'turns=100')
    await page.evaluate(() => {
      window.__transcript?.setWorking(true)
      window.__transcript?.startBatch('live', 4)
    })
    await settle(page)
    const group = `${rowSelector('fc_live-0')} [data-function-call-group]`
    const head = page.locator(`${group} .chat-tool-group__head`)
    await page.evaluate(() => window.__transcript?.settleBatch('live'))
    await settle(page)
    // Closed and reopened by hand before it was over: that open is theirs.
    await head.click()
    await expect(page.locator(group)).toHaveAttribute('data-expanded', 'false')
    await head.click()
    await expect(page.locator(group)).toHaveAttribute('data-expanded', 'true')
    // Focus moves on, to the composer say: the choice has to outlive it.
    await page.evaluate(() =>
      (document.activeElement as HTMLElement | null)?.blur(),
    )
    await page.evaluate(() =>
      window.__transcript?.stream('That covers those calls.'),
    )
    await settle(page)
    await expect(page.locator(group)).toHaveAttribute('data-expanded', 'true')
  })

  test('a call the reader opened keeps its batch open', async ({ page }) => {
    await open(page, 'turns=100')
    await page.evaluate(() => {
      window.__transcript?.setWorking(true)
      window.__transcript?.startBatch('live', 4)
    })
    await settle(page)
    await page.evaluate(() => window.__transcript?.settleBatch('live'))
    await settle(page)
    const group = `${rowSelector('fc_live-0')} [data-function-call-group]`
    const toggle = page.locator(
      `${group} [data-message-row="live-1"] [data-message-action="toggle"]`,
    )
    await toggle.click()
    await expect(toggle).toHaveAttribute('aria-expanded', 'true')
    // Focus moves on, to the composer say: the choice has to outlive it.
    await page.evaluate(() =>
      (document.activeElement as HTMLElement | null)?.blur(),
    )
    await page.evaluate(() =>
      window.__transcript?.stream('That covers those calls.'),
    )
    await settle(page)
    await expect(page.locator(group)).toHaveAttribute('data-expanded', 'true')
    await expect(toggle).toHaveAttribute('aria-expanded', 'true')
  })

  test('answering an approval does not keep the batch open', async ({
    page,
  }) => {
    await open(page, 'turns=100')
    await page.evaluate(() => {
      window.__transcript?.setWorking(true)
      window.__transcript?.startBatch('live', 3)
      window.__transcript?.holdCall('live-1')
    })
    await settle(page)
    const group = `${rowSelector('fc_live-0')} [data-function-call-group]`
    await page.locator(`${group} [data-message-action="approve"]`).click()
    await page.evaluate(() => {
      window.__transcript?.settleBatch('live')
      window.__transcript?.stream('That covers those calls.')
    })
    await expect(page.locator(group)).toHaveAttribute('data-expanded', 'false')
  })

  test('text selected in a batch keeps it open', async ({ page }) => {
    await open(page, 'turns=100')
    await page.evaluate(() => {
      window.__transcript?.setWorking(true)
      window.__transcript?.startBatch('live', 4)
    })
    await settle(page)
    await page.evaluate(() => window.__transcript?.settleBatch('live'))
    await settle(page)
    const group = `${rowSelector('fc_live-0')} [data-function-call-group]`
    await page.evaluate((selector) => {
      const node = document.querySelector(selector)
      if (node) getSelection()?.selectAllChildren(node)
    }, `${group} [data-message-row="live-1"]`)
    await page.evaluate(() =>
      window.__transcript?.stream('That covers those calls.'),
    )
    await settle(page)
    await expect(page.locator(group)).toHaveAttribute('data-expanded', 'true')
    expect(await page.evaluate(() => getSelection()?.isCollapsed)).toBe(false)
  })

  test('a landing on a hidden call keeps its batch open', async ({ page }) => {
    await open(page, 'turns=100')
    // Turn 51's batch has three calls, so a settled one hides the first.
    await expect(page.locator('[data-message-row="call-51-0"]')).toHaveCount(0)
    await page.evaluate(() => window.__transcript?.focus('call-51-0'))
    await settle(page)
    await page.waitForTimeout(400)
    await expect(
      page.locator('[data-message-row="call-51-0"]'),
    ).toBeInViewport()
  })

  test('a batch does not fold under a paused reader looking at it', async ({
    page,
  }) => {
    await open(page, 'turns=100')
    await page.evaluate(() => {
      window.__transcript?.setWorking(true)
      window.__transcript?.startBatch('live', 12)
    })
    await settle(page)
    const key = 'fc_live-0'
    const group = `${rowSelector(key)} [data-function-call-group]`
    await page.hover('[data-message-list]')
    await page.mouse.wheel(0, -120)
    await settle(page)
    expect((await metrics(page)).tail).toBe('paused')
    const before = await rowState(page, key)
    await page.evaluate(() => {
      window.__transcript?.settleBatch('live')
      window.__transcript?.stream('That covers those calls.')
    })
    await settle(page)
    await expect(page.locator(group)).toHaveAttribute('data-expanded', 'true')
    const after = await rowState(page, key)
    expect(Math.abs((after?.top ?? 0) - (before?.top ?? 0))).toBeLessThan(2)
    // Out of the window and back, nobody is reading it any more: it is folded,
    // as it would be after a reload.
    await page.evaluate(() => window.__transcript?.append(40))
    await settle(page)
    await scrollTo(page, 0)
    await expect(page.locator(rowSelector(key))).toHaveCount(0)
    await scrollUntilMounted(page, key)
    await expect(page.locator(group)).toHaveAttribute('data-expanded', 'false')
  })

  test('a batch that mounts settled does not replay its entrance', async ({
    page,
  }) => {
    await open(page, 'turns=100')
    await scrollTo(page, 0)
    const names = await page.evaluate(() => {
      const icon = document.querySelector('.chat-tool-group__icon')
      return icon ? getComputedStyle(icon).animationName : 'missing'
    })
    expect(names).toBe('none')
  })

  test('a pending approval far above stays mounted and reachable', async ({
    page,
  }) => {
    await open(page, 'turns=200')
    await page.evaluate(() => window.__transcript?.holdForApproval(2))
    await settle(page)
    await expect(page.locator('[data-approval-actions]')).toHaveCount(1)
    await scrollTo(page, 1e9)
    await expect(page.locator('[data-approval-actions]')).toHaveCount(1)
    expect((await metrics(page)).rows).toBeLessThan(MAX_MOUNTED_ROWS)
    await scrollTo(page, Math.floor((await metrics(page)).scrollHeight / 2))
    await page.getByRole('button', { name: 'jump to action required' }).click()
    await settle(page)
    await expect(page.locator('[data-approval-actions]')).toBeInViewport()
    // Its first enabled action, or the card itself when none is enabled.
    expect(
      await page.evaluate(
        () =>
          !!document.activeElement
            ?.closest('[data-message-id]')
            ?.querySelector('[data-approval-actions]'),
      ),
    ).toBe(true)
  })

  test('a far landing reaches a row never mounted, tall rows or not', async ({
    page,
  }) => {
    await open(page, 'turns=200')
    // reply-36 is 150 repeats tall, and so are its neighbors every 9 turns.
    for (const key of ['reply-36', 'reply-3', 'reply-120']) {
      await expect(page.locator(rowSelector(key))).toHaveCount(0)
      await page.evaluate((id) => window.__transcript?.focus(id), key)
      await settle(page)
      await expect(page.locator(rowSelector(key))).toBeInViewport()
      expect((await metrics(page)).tail).toBe('paused')
      await scrollTo(page, 1e9)
    }
  })

  test('a near upward landing is not cancelled by rows above it', async ({
    page,
  }) => {
    await open(page, 'turns=200')
    const target = await page.evaluate((row) => {
      const list = document.querySelector<HTMLElement>('[data-message-list]')
      const box = list?.getBoundingClientRect()
      const rows = [...document.querySelectorAll<HTMLElement>(row)]
      const near = rows.filter((el) => {
        const rect = el.getBoundingClientRect()
        return box && rect.bottom < box.top - 200
      })
      return near[near.length - 1]?.dataset.messageRow
    }, ROW)
    expect(target).toBeTruthy()
    await page.evaluate((id) => window.__transcript?.focus(id ?? null), target)
    await settle(page)
    await expect(page.locator(rowSelector(target as string))).toBeInViewport()
  })

  test('a selected row is held until the selection ends', async ({ page }) => {
    await open(page, 'turns=100')
    const key = 'progress-99'
    await expect(page.locator(rowSelector(key))).toHaveCount(1)
    await page.evaluate((selector) => {
      const node = document.querySelector(selector)
      if (node) getSelection()?.selectAllChildren(node)
    }, rowSelector(key))
    await settle(page)
    await scrollTo(page, 0)
    await expect(page.locator(rowSelector(key))).toHaveCount(1)
    expect(await page.evaluate(() => getSelection()?.isCollapsed)).toBe(false)
    await page.evaluate(() => getSelection()?.removeAllRanges())
    await settle(page)
    await expect(page.locator(rowSelector(key))).toHaveCount(0)
  })

  test('a batch keeps its tight spacing at the edge of the window', async ({
    page,
  }) => {
    await open(page, 'turns=100')
    const groups = await page.evaluate((row) => {
      const stack = document.querySelector('.chat-message-stack')
      const rows = [...(stack?.querySelectorAll<HTMLElement>(row) ?? [])]
      return rows
        .filter((el) => el.querySelector(':scope > [data-function-call-group]'))
        .map((el, position) => ({
          position,
          // A row with no mounted neighbor above it is the case that matters.
          edge:
            el.previousElementSibling?.hasAttribute('data-transcript-spacer') ??
            false,
          margin: getComputedStyle(
            el.querySelector(':scope > [data-function-call-group]') as Element,
          ).marginTop,
        }))
    }, ROW)
    expect(groups.length).toBeGreaterThan(0)
    // Every batch in this transcript follows prose: all tight, edge or not.
    for (const group of groups) {
      expect(Number.parseFloat(group.margin)).toBeLessThan(0)
    }
  })

  test('opening a card at the top of the view does not scroll it away', async ({
    page,
  }) => {
    await open(page, 'turns=100')
    const { scrollHeight } = await metrics(page)
    let placed: string | null = null
    // Tall replies can fill the view: walk until a batch that can open is in it.
    for (let top = scrollHeight / 2; top > 0 && !placed; top -= 700) {
      await scrollTo(page, Math.floor(top))
      placed = await page.evaluate((row) => {
        const list = document.querySelector<HTMLElement>('[data-message-list]')
        const box = list?.getBoundingClientRect()
        const rows = [...document.querySelectorAll<HTMLElement>(row)]
        const target = rows.find((el) => {
          const head = el.querySelector<HTMLElement>(
            '[data-can-collapse="true"] > .chat-tool-group__head',
          )
          const rect = head?.getBoundingClientRect()
          return rect && box && rect.top > box.top && rect.bottom < box.bottom
        })
        const head = target?.querySelector<HTMLElement>(
          '.chat-tool-group__head',
        )
        if (!list || !box || !target || !head) return null
        // Just inside the top edge: the row's bottom is what is still in view.
        list.scrollTop += head.getBoundingClientRect().top - box.top - 4
        return target.dataset.messageRow as string
      }, ROW)
    }
    expect(placed).toBeTruthy()
    await settle(page)
    const head = page.locator(
      `${rowSelector(placed as string)} .chat-tool-group__head`,
    )
    const before = await head.boundingBox()
    await head.click()
    await settle(page)
    const after = await head.boundingBox()
    expect(Math.abs((after?.y ?? 0) - (before?.y ?? 0))).toBeLessThan(2)
  })

  test('scrolling away with no gesture is not pulled back to the tail', async ({
    page,
  }) => {
    await open(page, 'turns=200')
    await page.evaluate(() => {
      const list = document.querySelector<HTMLElement>('[data-message-list]')
      if (list) list.scrollTop = Math.floor(list.scrollHeight / 2)
    })
    await settle(page)
    await settle(page)
    const m = await metrics(page)
    expect(m.tail).toBe('paused')
    expect(m.scrollHeight - m.clientHeight - m.scrollTop).toBeGreaterThan(
      m.clientHeight,
    )
  })

  test('a row grown in a hidden tab leaves with its real height', async ({
    page,
  }) => {
    // A hidden document runs no rendering steps, so ResizeObserver stays silent.
    await page.addInitScript(() => {
      const Native = window.ResizeObserver
      window.ResizeObserver = class extends Native {
        constructor(callback: ResizeObserverCallback) {
          super((entries, observer) => {
            if (!window.__hiddenTab) callback(entries, observer)
          })
        }
      }
      Object.defineProperty(document, 'hidden', {
        get: () => window.__hiddenTab === true,
      })
    })
    await open(page, 'turns=60')
    // Nothing moves the window while hidden, so a row only leaves it if it
    // starts past the window and is held there by being live.
    await page.evaluate(() => {
      window.__hiddenTab = true
      window.__transcript?.append(3)
    })
    await settle(page)
    await page.evaluate(() => window.__transcript?.stream('Short. '))
    await settle(page)
    const key = await page.evaluate((row) => {
      const rows = document.querySelectorAll<HTMLElement>(row)
      return rows[rows.length - 1].dataset.messageRow as string
    }, ROW)
    for (let token = 0; token < 30; token++) {
      await page.evaluate(() =>
        window.__transcript?.stream(
          'A long reply keeps growing here. '.repeat(120),
        ),
      )
      await settle(page)
    }
    const height = await page
      .locator(rowSelector(key))
      .evaluate((el) => el.getBoundingClientRect().height)
    expect(height).toBeGreaterThan(10000)
    const before = (await metrics(page)).scrollHeight
    await page.evaluate(() => {
      window.__transcript?.finishStream()
      window.__transcript?.append(1)
    })
    await settle(page)
    await expect(page.locator(rowSelector(key))).toHaveCount(0)
    // Judged while still hidden: back in view the row can re-enter the window
    // and correct itself. Its spacer must hold the height it reached, not the
    // one it mounted with, so the transcript does not collapse under the reader.
    expect((await metrics(page)).scrollHeight).toBeGreaterThan(before - 100)
  })
})

test.describe('sidebar window', () => {
  const LIST = '.conversation-list-scroll'
  const ITEM = '.conversation-tree > [data-index]'
  const chat = (index: number) =>
    `[aria-label="open Chat ${index} about something long enough to fade"]`
  /** Viewport (about 750px of 37px rows) plus overscan on both sides. */
  const MAX_MOUNTED_ITEMS = 70

  const openList = async (page: Page, count: number) => {
    await page.goto(`/transcript-stress.html?sidebar=${count}`)
    await page.waitForSelector(ITEM)
    await settle(page)
  }
  const listMetrics = (page: Page) =>
    page.evaluate(
      ({ list, item }) => {
        const scroller = document.querySelector<HTMLElement>(list)
        if (!scroller) throw new Error('no list')
        const box = scroller.getBoundingClientRect()
        const items = [...scroller.querySelectorAll(item)]
        return {
          items: items.length,
          nodes: scroller.querySelectorAll('*').length,
          spacers: scroller.querySelectorAll('[data-list-spacer]').length,
          scrollHeight: scroller.scrollHeight,
          clientHeight: scroller.clientHeight,
          inView: items.filter((el) => {
            const rect = el.getBoundingClientRect()
            return rect.bottom > box.top && rect.top < box.bottom
          }).length,
        }
      },
      { list: LIST, item: ITEM },
    )
  const scrollListTo = async (page: Page, top: number) => {
    await page.evaluate(
      ({ list, y }) => {
        const scroller = document.querySelector<HTMLElement>(list)
        if (scroller) scroller.scrollTop = y
      },
      { list: LIST, y: top },
    )
    await settle(page)
  }

  test('a short list mounts every row and needs no spacer', async ({
    page,
  }) => {
    await openList(page, 5)
    expect((await listMetrics(page)).spacers).toBe(0)
    await expect(page.locator(chat(4))).toHaveCount(1)
  })

  test('a long list mounts only what is around the viewport', async ({
    page,
  }) => {
    await openList(page, 400)
    const m = await listMetrics(page)
    expect(m.items).toBeLessThan(MAX_MOUNTED_ITEMS)
    expect(m.spacers).toBeGreaterThan(0)
    // Still as long as the whole list: the scrollbar tells the truth.
    expect(m.scrollHeight).toBeGreaterThan(400 * 36)
  })

  for (const grouping of ['date', 'project'] as const)
    test(`the viewport stays full and steady all the way down (${grouping})`, async ({
      page,
    }) => {
      // By project, every row after a section's first is stacked 1px up.
      if (grouping === 'project')
        await page.addInitScript(() => {
          localStorage.setItem(
            'iii-chat-list-view',
            JSON.stringify({
              kinds: ['user'],
              grouping: 'project',
              sort: 'updated',
            }),
          )
        })
      await openList(page, 300)
      await page.hover(LIST)
      const heights = new Set<number>()
      for (let step = 0; step < 40; step++) {
        const before = await page.evaluate(
          ({ list, item }) => {
            const scroller = document.querySelector<HTMLElement>(list)
            const box = scroller?.getBoundingClientRect()
            const node = [
              ...(scroller?.querySelectorAll<HTMLElement>(item) ?? []),
            ].find((el) => box && el.getBoundingClientRect().top >= box.top)
            return node
              ? {
                  index: node.dataset.index as string,
                  top: node.getBoundingClientRect().top,
                }
              : null
          },
          { list: LIST, item: ITEM },
        )
        expect(before).not.toBeNull()
        await page.mouse.wheel(0, 300)
        await settle(page)
        const top = await page.evaluate(
          ({ item, index }) =>
            document
              .querySelector(`${item}[data-index="${index}"]`)
              ?.getBoundingClientRect().top,
          { item: ITEM, index: before?.index },
        )
        const m = await listMetrics(page)
        heights.add(m.scrollHeight)
        expect(m.inView).toBeGreaterThan(10)
        if (top !== undefined && m.scrollHeight - m.clientHeight > 0) {
          const moved = (before?.top ?? 0) - top
          expect(moved === 0 || Math.abs(moved - 300) < 1).toBe(true)
        }
      }
      // Every row here is its estimate: the list is as long wherever it is
      // scrolled, so each spacer stands in for exactly the rows it replaces.
      expect([...heights]).toHaveLength(1)
      await scrollListTo(page, 1e9)
      await expect(page.locator(ITEM).last()).toBeInViewport()
      expect((await listMetrics(page)).items).toBeLessThan(MAX_MOUNTED_ITEMS)
    })

  test('the active chat stays mounted out of view', async ({ page }) => {
    await openList(page, 300)
    await page.locator(chat(2)).click()
    // Focus elsewhere, so only being active holds it (focus holds neighbors too).
    await page.locator(chat(8)).focus()
    await scrollListTo(page, 1e9)
    await expect(page.locator(chat(3))).toHaveCount(0)
    await expect(page.locator(chat(2))).toHaveAttribute('aria-current', 'page')
  })

  test('the focused chat keeps its focus out of view', async ({ page }) => {
    await openList(page, 300)
    await page.locator(chat(4)).focus()
    await scrollListTo(page, 1e9)
    await expect(page.locator(chat(2))).toHaveCount(0)
    await expect(page.locator(chat(4))).toBeFocused()
    // Its neighbors are held too, so the keyboard walk goes on from it.
    await expect(page.locator(chat(3))).toHaveCount(1)
    await page.keyboard.press('Shift+Tab')
    expect(
      await page.evaluate(
        (selector) =>
          !!document
            .querySelector(selector)
            ?.parentElement?.contains(document.activeElement),
        chat(3),
      ),
    ).toBe(true)
  })

  test('Tab walks on past the edge of the window', async ({ page }) => {
    await openList(page, 300)
    await page.locator(chat(0)).focus()
    for (let step = 0; step < 90; step++) await page.keyboard.press('Tab')
    const inList = await page.evaluate(
      ({ item }) => !!document.activeElement?.closest(item),
      { item: ITEM },
    )
    expect(inList).toBe(true)
    await expect(page.locator(':focus')).toBeInViewport()
  })

  test('a dragged chat stays until the drag ends', async ({ page }) => {
    await page.addInitScript(() => {
      localStorage.setItem(
        'iii-chat-pinned',
        JSON.stringify(['chat-40', 'chat-41', 'chat-42']),
      )
    })
    await openList(page, 300)
    const dragged = chat(40)
    await page.evaluate((selector) => {
      document.querySelector(selector)?.dispatchEvent(
        new DragEvent('dragstart', {
          bubbles: true,
          dataTransfer: new DataTransfer(),
        }),
      )
    }, dragged)
    await settle(page)
    await scrollListTo(page, 1e9)
    await expect(page.locator(chat(41))).toHaveCount(0)
    await expect(page.locator(dragged)).toHaveCount(1)
    await page.evaluate((selector) => {
      document
        .querySelector(selector)
        ?.dispatchEvent(new DragEvent('dragend', { bubbles: true }))
    }, dragged)
    await settle(page)
    await expect(page.locator(dragged)).toHaveCount(0)
  })

  test('collapsing a section takes its rows out of the list', async ({
    page,
  }) => {
    await openList(page, 300)
    const before = await listMetrics(page)
    await page
      .getByRole('button', { name: /^Today/ })
      .first()
      .click()
    await settle(page)
    const after = await listMetrics(page)
    expect(after.scrollHeight).toBeLessThan(before.scrollHeight - 200)
    expect(after.inView).toBeGreaterThan(10)
  })

  test('a chat jumping to the top does not move what the reader sees', async ({
    page,
  }) => {
    await openList(page, 300)
    await scrollListTo(page, 4000)
    const key = await page.evaluate(
      ({ list, item }) => {
        const scroller = document.querySelector<HTMLElement>(list)
        const box = scroller?.getBoundingClientRect()
        const node = [
          ...(scroller?.querySelectorAll<HTMLElement>(item) ?? []),
        ].find(
          (el) =>
            box &&
            el.getBoundingClientRect().top >= box.top + 40 &&
            el.querySelector('[aria-label^="open "]'),
        )
        const label = node?.querySelector('[aria-label^="open "]')
        return label
          ? {
              label: label.getAttribute('aria-label') as string,
              top: label.getBoundingClientRect().top,
            }
          : null
      },
      { list: LIST, item: ITEM },
    )
    expect(key).not.toBeNull()
    await page.evaluate(() => window.__sidebar?.touch('chat-290'))
    await settle(page)
    const top = await page.evaluate(
      (label) =>
        document
          .querySelector(`[aria-label="${label}"]`)
          ?.getBoundingClientRect().top,
      key?.label as string,
    )
    expect(Math.abs((top ?? 0) - (key?.top ?? 0))).toBeLessThan(2)
  })

  test('date headers keep their margin, except the first', async ({ page }) => {
    await openList(page, 60)
    const margins = await page.evaluate(
      (item) =>
        [...document.querySelectorAll<HTMLElement>(item)]
          .map((el) => el.firstElementChild as HTMLElement)
          .filter((el) => el.classList.contains('conversation-group-header'))
          .map((el) => getComputedStyle(el).marginTop),
      ITEM,
    )
    expect(margins[0]).toBe('0px')
    expect(margins.length).toBeGreaterThan(1)
    expect(margins[1]).toBe('8px')
  })

  test('a chat changing section keeps its open dialog', async ({ page }) => {
    await openList(page, 300)
    // 26 hours old: an older bucket, and new activity moves it to Today.
    await page.locator(chat(26)).dblclick()
    const dialog = page.getByRole('dialog')
    await expect(dialog).toBeVisible()
    await page.keyboard.press('ControlOrMeta+a')
    await page.keyboard.type('Draft title')
    await page.evaluate(() => window.__sidebar?.touch('chat-26'))
    // Past any exit animation: a remounted row would have closed it by now.
    await page.waitForTimeout(800)
    await expect(dialog).toBeVisible()
    await expect(dialog.getByRole('textbox')).toHaveValue('Draft title')
  })

  test('rows taller than estimated scroll up without a jump', async ({
    page,
  }) => {
    // A larger root font, as zoom or a reader's setting gives: rem rows outgrow
    // the pixel estimate, and every row measured on the way up corrects it.
    await page.addInitScript(() => {
      // Init scripts run before <html> exists.
      document.addEventListener('DOMContentLoaded', () => {
        document.documentElement.style.fontSize = '22px'
      })
    })
    await openList(page, 300)
    await scrollListTo(page, 1e9)
    await page.hover(LIST)
    for (let step = 0; step < 30; step++) {
      const before = await page.evaluate(
        ({ list, item }) => {
          const scroller = document.querySelector<HTMLElement>(list)
          const box = scroller?.getBoundingClientRect()
          const node = [
            ...(scroller?.querySelectorAll<HTMLElement>(item) ?? []),
          ].find((el) => box && el.getBoundingClientRect().top >= box.top)
          return node
            ? {
                index: node.dataset.index as string,
                top: node.getBoundingClientRect().top,
              }
            : null
        },
        { list: LIST, item: ITEM },
      )
      await page.mouse.wheel(0, -250)
      await settle(page)
      const top = await page.evaluate(
        ({ item, index }) =>
          document
            .querySelector(`${item}[data-index="${index}"]`)
            ?.getBoundingClientRect().top,
        { item: ITEM, index: before?.index },
      )
      expect(top, `row ${before?.index} left at step ${step}`).toBeDefined()
      const moved = (top ?? 0) - (before?.top ?? 0)
      expect(moved === 0 || Math.abs(moved - 250) < 1).toBe(true)
    }
  })

  test('a section moving up keeps the row in view', async ({ page }) => {
    await page.addInitScript(() => {
      localStorage.setItem(
        'iii-chat-list-view',
        JSON.stringify({
          kinds: ['user'],
          grouping: 'project',
          sort: 'updated',
        }),
      )
    })
    await openList(page, 300)
    await scrollListTo(page, 6000)
    const viewed = await page.evaluate(
      ({ list, item }) => {
        const scroller = document.querySelector<HTMLElement>(list)
        const box = scroller?.getBoundingClientRect()
        const label = [...(scroller?.querySelectorAll<HTMLElement>(item) ?? [])]
          .filter((el) => box && el.getBoundingClientRect().top >= box.top + 40)
          .map((el) => el.querySelector('[aria-label^="open Chat"]'))
          .find(Boolean)
        const name = label?.getAttribute('aria-label')
        const index = Number(/Chat (\d+)/.exec(name ?? '')?.[1])
        return label && name
          ? { name, index, top: label.getBoundingClientRect().top }
          : null
      },
      { list: LIST, item: ITEM },
    )
    expect(viewed).not.toBeNull()
    // The project's newest chat gets activity, so the whole section moves to the
    // top of the list, far more rows than the window keeps around the viewed one.
    const project = (viewed?.index ?? 0) % 7
    expect(project).not.toBe(0)
    expect(viewed?.index).toBeGreaterThan(project)
    await page.evaluate((id) => window.__sidebar?.touch(id), `chat-${project}`)
    await settle(page)
    const top = await page.evaluate(
      (name) =>
        document
          .querySelector(`[aria-label="${name}"]`)
          ?.getBoundingClientRect().top,
      viewed?.name as string,
    )
    expect(Math.abs((top ?? -1e9) - (viewed?.top ?? 0))).toBeLessThan(2)
  })

  test('project sections keep their nesting and rhythm', async ({ page }) => {
    await page.addInitScript(() => {
      localStorage.setItem(
        'iii-chat-list-view',
        JSON.stringify({
          kinds: ['user'],
          grouping: 'project',
          sort: 'updated',
        }),
      )
    })
    await openList(page, 300)
    const shape = await page.evaluate((item) => {
      const items = [...document.querySelectorAll<HTMLElement>(item)]
      const firstHeader = items[0].querySelector('.conversation-project-header')
      const rows = items.filter((el) =>
        el.classList.contains('conversation-project-rows'),
      )
      const inner = (el: Element) =>
        (el.firstElementChild as Element).getBoundingClientRect()
      return {
        firstMargin: firstHeader
          ? getComputedStyle(firstHeader).marginTop
          : 'missing',
        // Rows of one project touch, as they did inside their shared wrapper.
        rowGap: inner(rows[1]).top - inner(rows[0]).bottom,
        inset: getComputedStyle(rows[0]).getPropertyValue(
          '--iii-ui-tree-inset',
        ),
      }
    }, ITEM)
    expect(shape.firstMargin).toBe('0px')
    expect(shape.rowGap).toBe(0)
    expect(shape.inset).toBe('calc(10px + 14px)')
    // A project's section outruns the window: walk to the next heading.
    let margin: string | null = null
    for (let top = 400; top < 12000 && !margin; top += 400) {
      await scrollListTo(page, top)
      margin = await page.evaluate((item) => {
        const header = [...document.querySelectorAll<HTMLElement>(item)]
          .filter((el) => Number(el.dataset.index) > 0)
          .map((el) => el.querySelector('.conversation-project-header'))
          .find(Boolean)
        return header ? getComputedStyle(header).marginTop : null
      }, ITEM)
    }
    expect(margin).toBe('12px')
  })
})
