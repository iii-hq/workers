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
 * @typedef {{ type: string, config: Record<string, unknown>, label: string, hint?: string, prompt?: string }} Condition
 * @typedef {{ id: string, title: string, body: string, anchors?: string[], condition?: Condition, screen?: string }} Step
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
        id: 'composer',
        title: 'Send a message',
        body: 'Type here. The harness is a worker, the same as every other worker. It picks a model through llm-router, then it calls functions that other workers register. All workers speak one interface: Workers, Triggers, Functions. This is why any worker can use any other worker.',
        anchors: ['.onboarding-composer', '.composer-shell'],
        condition: {
          type: 'harness::turn-completed',
          config: {},
          label: 'Waiting for a harness turn to finish',
          hint: 'Send any message in a chat pane.',
        },
      },
      {
        id: 'welcome',
        title: 'This is an engine, not a chat app',
        body: 'Each panel on this page is a live client of one iii engine. The engine orchestrates; the workers extend what the system can do. A worker is a running service, not a plugin: it starts, it stays up, and it registers its functions and triggers with the engine. The console shows all of them as one system.',
        anchors: ['.onboarding-menu-bar', 'header.h-14'],
      },
      {
        id: 'traces',
        title: 'Watch the work happen',
        body: 'Open Traces to see the message you just sent. Each function call and each trigger writes a span, so a trace shows which worker ran, in which order, and how long each part took. The iii-observability worker collects the spans and can export them to any OpenTelemetry backend.',
        anchors: ['.onboarding-traces', 'section[aria-label="traces"]'],
        // The step's button places this console screen beside the tour, so the
        // operator reads the trace instead of hunting for the palette row.
        screen: 'traces',
      },
      {
        id: 'triggers',
        title: 'Triggers watch for you',
        body: 'A trigger binds an event to a function, so the system reacts instead of polls. This step is bound to the `state` trigger type. It fires when anything writes to the `tour-scratch` scope, and the card below shows the event as the engine delivered it.',
        anchors: ['.onboarding-composer', '.composer-shell'],
        condition: {
          type: 'state',
          config: { scope: 'tour-scratch' },
          label: 'Waiting for a write to the tour-scratch scope',
          hint: 'iii trigger state::set scope=tour-scratch key=hello value=world',
          prompt: 'Use the state worker to set key "hello" to "world" in the tour-scratch scope.',
        },
      },
      {
        id: 'tabs',
        title: 'Workspaces, not windows',
        body: 'Each tab is a workspace of one or more panes, side by side. Split a tab to keep a chat beside a page that a worker injected — this onboarding page is one of those. Install more workers from the package repo, or write your own, and they add functions, commands and pages to this same console.',
        anchors: ['.onboarding-tabs', '[role="tablist"][aria-label="Workspace tabs"]'],
      },
      {
        id: 'palette',
        title: 'One key reaches everything',
        body: 'The command palette lists each page, command and worker-provided row. Workers add their rows while they run, so the palette grows when you install a worker. One list, one system.',
        anchors: ['.onboarding-palette', 'button[aria-label^="Search and commands"]'],
      },
      {
        id: 'conversations',
        title: 'Conversations are sessions',
        body: 'Each conversation is a harness session with its own history, working directory and model. The engine holds that state, not the browser: close the tab, come back, and the session is where you left it.',
        anchors: ['.onboarding-conversations', 'aside[aria-label="Conversations"]'],
      },
      {
        id: 'stay-in-touch',
        title: 'Stay in touch',
        body: 'iii moves quickly. Put your email in for the roadmap and the product updates — what is being built, and what shipped. The links below go to the same places, if you would rather read along there.',
        // No anchor: this step is about the page itself, and there is nothing
        // in the console to point at. The page renders the signup box and the
        // links for this step id.
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
