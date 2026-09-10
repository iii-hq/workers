/**
 * Tour content. One tour is a title plus an ordered list of steps; one step is
 * a headline, a short body, and the console element it talks about.
 *
 * `anchor` is a CSS selector for a class the console carries FOR THIS TOUR
 * (`onboarding-*`, added in console/web/src). The injected page draws its
 * spotlight around the first match, and skips the box when the selector
 * matches nothing — a step whose element is off screen still reads fine.
 */

/** @typedef {{ id: string, title: string, body: string, anchor?: string }} Step */
/** @typedef {{ id: string, title: string, description: string, steps: Step[] }} Tour */

/** @type {Tour[]} */
export const TOURS = [
  {
    id: 'console-basics',
    title: 'Find your way around',
    description: 'The five surfaces of the console, and what each one is for.',
    steps: [
      {
        id: 'welcome',
        title: 'This is your engine',
        body: 'Everything on this page talks to one iii engine over a WebSocket. The engine holds workers; workers register functions and triggers. Nothing here is a static page — each panel is a live client of the same engine.',
        anchor: '.onboarding-menu-bar',
      },
      {
        id: 'tabs',
        title: 'Workspaces, not windows',
        body: 'Each tab is a workspace: one or more panes, side by side. Split a tab to keep a chat next to a page that a worker injected. Tabs and their panes persist across reloads, so a layout you like stays put.',
        anchor: '.onboarding-tabs',
      },
      {
        id: 'palette',
        title: 'One key reaches everything',
        body: 'The command palette lists every page, command, and worker-provided row. Workers add their own rows at runtime, so the palette grows as you install workers. It is the fastest route to anything in the console.',
        anchor: '.onboarding-palette',
      },
      {
        id: 'conversations',
        title: 'Conversations are sessions',
        body: 'Each conversation is a harness session with its own history, working directory, and model. The engine owns that state, not the browser: close the tab, come back, and the session is where you left it.',
        anchor: '.onboarding-conversations',
      },
      {
        id: 'composer',
        title: 'Where work starts',
        body: 'Type here and the harness picks a model through llm-router, then calls functions on your behalf. Attach files, pick a folder, or point it at a worker — the tools it can reach are the functions registered in your engine.',
        anchor: '.onboarding-composer',
      },
      {
        id: 'settings',
        title: 'Configuration is a function call',
        body: 'Every configurable worker publishes a schema, and this panel edits the value. Saving writes it through the engine, so the worker sees the change without a restart. Provider keys live here too.',
        anchor: '.onboarding-settings',
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
