/**
 * Tour content. One tour is a title plus an ordered list of steps.
 *
 * A step carries:
 *   - `anchors`: selectors for the console element the step talks about, best
 *     first. The first one is always the `onboarding-*` class the console
 *     carries FOR THIS TOUR (added in console/web/src, and checked by
 *     tests/); the ones after it are the console's own stable hooks, so the
 *     spotlight still lands on a console build that predates the anchor
 *     classes.
 *   - `condition` (optional): a real engine trigger the step waits for. The
 *     page binds it, shows what it is waiting for, and when it fires shows
 *     the trigger and its payload. A step with no condition is closed by the
 *     operator with a button.
 */

/**
 * @typedef {{ type: string, config: Record<string, unknown>, label: string, hint?: string }} Condition
 * @typedef {{ id: string, title: string, body: string, anchors?: string[], condition?: Condition }} Step
 * @typedef {{ id: string, title: string, description: string, steps: Step[] }} Tour
 */

/** @type {Tour[]} */
export const TOURS = [
  {
    id: 'console-basics',
    title: 'Find your way around',
    description: 'The surfaces of the console, and the engine underneath them.',
    steps: [
      {
        id: 'welcome',
        title: 'This is your engine',
        body: 'Everything on this page talks to one iii engine over a WebSocket. The engine holds workers; workers register functions and triggers. No panel here is a static page — each one is a live client of the same engine.',
        anchors: ['.onboarding-menu-bar', 'header.h-14'],
      },
      {
        id: 'tabs',
        title: 'Workspaces, not windows',
        body: 'Each tab is a workspace of one or more panes, side by side. Split a tab to keep a chat beside a page that a worker injected — this tour is one of those pages. Tabs and their panes persist across reloads.',
        anchors: ['.onboarding-tabs', '[role="tablist"][aria-label="Workspace tabs"]'],
      },
      {
        id: 'palette',
        title: 'One key reaches everything',
        body: 'The command palette lists every page, command, and worker-provided row. Workers add their rows at runtime, so the palette grows as you install workers.',
        anchors: ['.onboarding-palette', 'button[aria-label^="Search and commands"]'],
      },
      {
        id: 'conversations',
        title: 'Conversations are sessions',
        body: 'Each conversation is a harness session with its own history, working directory, and model. The engine owns that state, not the browser: close the tab, come back, and the session is where you left it.',
        anchors: ['.onboarding-conversations', 'aside[aria-label="Conversations"]'],
      },
      {
        id: 'composer',
        title: 'Send a message',
        body: 'Type here and the harness picks a model through llm-router, then calls functions on your behalf. The tools it can reach are the functions registered in your engine.',
        anchors: ['.onboarding-composer', '.composer-shell'],
        condition: {
          type: 'harness::turn-completed',
          config: {},
          label: 'Waiting for a harness turn to finish',
          hint: 'Send any message in a chat pane.',
        },
      },
      {
        id: 'triggers',
        title: 'Triggers watch for you',
        body: 'A trigger binds an event to a function. This step is bound to the `state` trigger type: it fires the moment anything is written to the `tour-scratch` scope, and the card below shows you the event as the engine delivered it.',
        anchors: ['.onboarding-composer', '.composer-shell'],
        condition: {
          type: 'state',
          config: { scope: 'tour-scratch' },
          label: 'Waiting for a write to the tour-scratch scope',
          hint: 'iii trigger state::set scope=tour-scratch key=hello value=world',
        },
      },
      {
        id: 'settings',
        title: 'Configuration is a function call',
        body: 'Every configurable worker publishes a schema, and this panel edits the value. Saving writes it through the engine, so the worker sees the change without a restart. Provider keys live here too.',
        anchors: ['.onboarding-settings', 'button[aria-label="console settings"]'],
      },
    ],
  },
]

/** Tours in curriculum order, each with its step count instead of its steps. */
export function listTours() {
  return TOURS.map((tour) => ({
    id: tour.id,
    title: tour.title,
    description: tour.description,
    step_count: tour.steps.length,
  }))
}

/** @returns {Tour | undefined} */
export function getTour(id) {
  return TOURS.find((tour) => tour.id === id)
}

/** @returns {Step | undefined} */
export function getStep(tourId, stepId) {
  return getTour(tourId)?.steps.find((step) => step.id === stepId)
}
