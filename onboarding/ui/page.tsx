import type { Host } from '@iii-dev/console-ui'
import { TourOverlay } from './src/overlay'
import { LearnIiiPage } from './src/page'
import { showTour } from './src/visibility'

/**
 * The tour floats over the console as a card where the console has the
 * overlay slot, and is a page in a pane where it does not. Either way
 * `Open learn-iii` is how it is shown — the palette row, and the command
 * the setup wizard runs when the tour is accepted or comes due.
 */
export default function setup(host: Host) {
  if (host.overlays?.register) {
    host.overlays.register({
      id: 'learn-iii-tour',
      render: (props) => <TourOverlay host={host} {...props} />,
    })
    void adoptOldPane(host)
  } else {
    host.pages.register({
      id: 'learn-iii',
      title: 'learn-iii',
      render: (props) => <LearnIiiPage host={host} {...props} />,
    })
  }
  const overlay = Boolean(host.overlays?.register)
  host.commands?.register('learn-iii', [
    {
      id: 'open',
      title: 'Open learn-iii',
      detail: 'Guided walkthrough of the console',
      keywords: ['learn-iii', 'learn iii', 'tour', 'walkthrough', 'help', 'tutorial'],
      run: () => (overlay ? showTour() : host.panels?.open({ pageId: 'learn-iii' })),
    },
  ])
}

/**
 * A workspace saved while the tour was a page still has its pane, which no
 * longer has a page to show. Close it and bring the tour up as the card, so
 * an operator mid-tour carries on instead of finding an empty pane. Best
 * effort: a console that cannot list or close panes just keeps the pane.
 */
async function adoptOldPane(host: Host): Promise<void> {
  try {
    const ws = await host.iii.trigger<{ tabs: { screens: string[] }[] }>('console::workspace::list')
    if (!ws.tabs.some((tab) => tab.screens.includes('ext:learn-iii'))) return
    await host.iii.trigger('console::workspace::close', { screen: 'ext:learn-iii' })
    showTour()
  } catch {
    // Nothing to adopt, or nothing that can do it.
  }
}
