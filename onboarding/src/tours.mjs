/**
 * Tour content. One tour is a title plus an ordered list of steps.
 *
 * A step carries:
 *   - `anchors`: selectors for the console element the step talks about, best
 *     first. The first one is always the `learn-iii-*` class the console
 *     carries FOR THIS TOUR (added in console/web/src, and checked by
 *     tests/); the ones after it are the console's own stable hooks, so the
 *     spotlight still lands on a console build that predates the anchor
 *     classes.
 *   - `ask` (optional): a prompt the step's button puts in the chat composer
 *     and sends, so the operator talks to the agent instead of copying text
 *     out of the tour. With `agent` (a profile id) the button starts a NEW
 *     chat under that profile and sends the prompt there, the way the setup
 *     wizard's example prompts do; without it the prompt goes to the chat
 *     the operator is in.
 *   - `to_close` (optional): a console screen the tour closes for the
 *     operator when the step opens and is not done yet, to make room.
 *   - `to_open` (optional): a console screen the step talks about. While it
 *     is on screen the step frames its own anchors; while it is not, the
 *     page shows this body, frames these anchors — where the screen opens by
 *     hand — and offers a button (`label`) that opens it beside the chat.
 *     Open only in a tab that is not showing, the page shows `elsewhere`
 *     and frames that tab.
 *   - `condition` (optional): a real engine trigger the step waits for. The
 *     page binds it, shows what it is waiting for, and when it fires shows
 *     the trigger and its payload. A step with no condition is closed by the
 *     operator with a button.
 */

/**
 * @typedef {{ type: string, config: Record<string, unknown>, label: string, hint?: string, prompt?: string }} Condition
 * @typedef {{ text: string, label: string, agent?: string }} Ask
 * @typedef {{ screen: string }} ToClose
 * @typedef {{ screen: string, label: string, body: string, anchors: string[], elsewhere?: string }} ToOpen
 * @typedef {{ id: string, title: string, body: string, anchors?: string[], condition?: Condition, ask?: Ask, to_close?: ToClose, to_open?: ToOpen }} Step
 * @typedef {{ id: "intro", title: string, body: string }} Intro
 * @typedef {{ id: string, title: string, description: string, intro: Intro, steps: Step[] }} Tour
 */

/**
 * The state scope the tour keeps everything in — progress, and the keys the
 * steps below wait on.
 */
const STATE_SCOPE = 'learn-iii'

/**
 * The agent profile the tour talks to: the one the harness template ships
 * for its setup examples (`agents/ade-worker-builder.md`, shown as
 * "Onboarding"). The first step starts a new chat under it; every later
 * prompt goes to the chat the operator is in, which is that one unless they
 * moved.
 */
export const TOUR_AGENT = 'ade-worker-builder'

/**
 * A step the AGENT finishes, not the operator.
 *
 * Its prompt ends by telling the agent to write `step_<id>_completed` into
 * the learn-iii state scope as it ENDS ITS TURN — not merely when it thinks
 * it is done, which it reports too early — and the step waits on a `state`
 * trigger bound to that one key. The page binds every reachable condition when the step is
 * DISPLAYED, before any button is clicked, so the step the operator is
 * looking at is the step the agent is still working on — a long build no
 * longer reads as finished the moment the prompt is sent.
 *
 * The trigger matches the KEY, not the value: whatever shape the agent
 * writes, the write itself is the report.
 *
 * `tours.test.mjs` holds the two halves together — the sentence and the
 * binding both carry the key, and the test fails when one of them moves.
 */
const agentStep = (step) => {
  const key = `step_${step.id}_completed`
  return {
    ...step,
    ask: {
      ...step.ask,
      text: `${step.ask.text} When you are ending your turn, write ${key}: true to state, scope "${STATE_SCOPE}".`,
    },
    condition: {
      type: 'state',
      config: { scope: STATE_SCOPE, key },
      label: 'Waiting for the agent to report this step done',
      hint: `The agent writes ${key} into the "${STATE_SCOPE}" state scope when it has finished.`,
    },
  }
}

/** @type {Tour[]} */
export const TOURS = [
  {
    id: "learn-iii",
    title: "Find your way around",
    description: "The surfaces of the console, and the engine underneath them.",
    // Step 0: the card asks before it teaches. "Show me" closes it (and is
    // what the funnel counts); "Not now" folds the card to its pill.
    intro: {
      id: "intro",
      title: "Learn iii?",
      body: "Want to learn more about iii, the engine that powers this harness and everything you build? A short tour shows you around the console.",
    },
    steps: [
      {
        id: "message",
        ask: {
          text: "hello",
          label: "Send a message",
          agent: TOUR_AGENT,
        },
        title: "Send a message",
        body: "Say 'hello' to the agent. The button opens a new chat with the Onboarding agent profile and sends it. We've set up a trigger to run when you get the agent's response. The iii ADE (agentic development environment) is an example of an iii Worker. Kanban and state are other examples of workers. Every Worker in the iii system can be composed, observed, and reacted to in the same way.",
        anchors: [".learn-iii-composer", ".composer-shell"],
        condition: {
          type: "harness::turn-completed",
          config: {},
          label: "Waiting for a harness turn to finish",
          hint: "Send any message in a chat pane.",
        },
      },
      {
        id: "tabs",
        title: "Workspaces",
        body: "Each tab is a workspace: one or more panes, side by side, that you arrange for one piece of work. The + at the end of the row opens another, and you can keep as many as you need.",
        // The tabs AND the + — the body talks about both, so the box takes in
        // both. The tab row alone is the fallback for an older console.
        anchors: [
          ".learn-iii-workspaces",
          ".learn-iii-tabs",
          '[role="tablist"][aria-label="Workspace tabs"]',
        ],
      },
      {
        id: "coder",
        title: "CODER",
        body: "The goal of iii is to be Composable, Observable, Discoverable, Extensible, and Reactive. We demonstrated a few of the properties in the last step. Now let's take a look at them one by one.",
        // No anchor: this step names the five properties, it does not point at
        // a console surface. The spotlight stays hidden.
      },
      agentStep({
        id: "composability",
        ask: {
          text: "Add a kanban board to this project in the todo-local namespace, the way the onboarding skill says (directory::skills::get { id: \"onboarding\" }, section 'Add the board'). When it runs, open its board page beside this chat and tell me what the board can do.",
          label: "Ask the agent",
        },
        title: "Composability",
        body: "iii is composable like Node or Python, except when you add to iii you add a working service and not a library. Ask the agent to add a kanban board. It runs in its own namespace, todo-local, so it sits apart from any board you already have.",
      }),
      {
        id: "stay-in-touch",
        title: "Stay in touch",
        body: "iii moves quickly. Put your email in for the roadmap and the product updates — what is being built, and what shipped. The links below go to the same places, if you would rather read along there.",
        // No anchor: this step is about the page itself, and there is nothing
        // in the console to point at. The page renders the signup box and the
        // links for this step id.
      },
      {
        id: "observability",
        title: "Observability",
        body: "Every function call and every trigger writes a span, so a trace shows which worker ran, in which order, and how long each part took. Open Traces: it lands beside the chat and selects the latest turn of the chat that is running, so the message you just sent is the first thing you see. The iii-observability worker collects the spans and can export them to any OpenTelemetry backend.",
        anchors: [".learn-iii-traces", 'section[aria-label="traces"]'],
        // The step's button opens Traces beside the chat. Until then the page
        // points at where it opens by hand too — the + first, then the traces
        // row of the new tab's picker (best match first, so the box moves on
        // once the picker is up). A phone has no tab row: its search button
        // is how Traces opens there.
        to_open: {
          screen: "traces",
          label: "Open traces",
          body: "Traces is not open yet. Open it with the button, or pick traces from a new tab's list.",
          elsewhere: "Traces is open in another tab. Switch to that tab, lit up in the tab row, to see it.",
          anchors: [
            '[role="option"][data-screen="traces"]',
            ".learn-iii-new-tab",
            '[aria-label="New workspace"]',
            '[aria-label="search the ADE"]',
          ],
        },
      },
      agentStep({
        id: "discoverability",
        ask: {
          text: "What can this system do right now? List the workers that are registered and the functions each one exposes.",
          label: "Ask the agent",
        },
        title: "Discoverability",
        body: "See what the system can do. Let's ask the agent what the current capabilities are.",
      }),
      agentStep({
        id: "extensibility",
        ask: {
          text: "Build a TODO worker on the todo-local kanban board, the way the onboarding skill says (directory::skills::get { id: \"onboarding\" }, section 'Build the TODO worker'): first set the board to two columns, TODO and Done; then a worker whose console page is a narrow sidebar of controls, not a list: an add form and an edit form with a due date, a due time, a priority, a repeat and an alarm per item, each shown as a pill on the board card. The board is the list and Done is set on the board; a done item with a repeat creates the next one; alarms go through the cron worker; a todo:changed trigger type; and the same controls as a web page served through the http worker. Verify through functions, not screenshots, and do not navigate me away from this workspace. When it is done, open the TODO page to the left of the board, and open the web page for me in the console's browser in a new tab (browser::sessions::start with preview: false, then console::workspace::open { screen: 'browser', placement: 'new-tab' }), not the preview window.",
          label: "Ask the agent",
        },
        title: "Extensibility",
        body: "Now build on that board. Ask the agent for a TODO worker: a sidebar of controls to the left of the board, with a due date, a due time, a priority, a repeat and an alarm on each item, and the same controls as a web page the agent opens for you. The board is the list and keeps the progress; the TODO worker adds the rest, and each setting shows as a pill on the card. The tour closes the Traces panel to make room for the TODO page; open it again from the palette any time.",
        // The step closes when the agent reports done; the panel goes as the
        // step opens, so the new page has room.
        to_close: { screen: "traces" },
      }),
      agentStep({
        id: "reactivity",
        ask: {
          text: "Add 3 items to the TODO list, each with a due date and a priority, and set a Trigger that fires when each one is checked off by the user, not you the agent. Pre-existing triggers should work, and were tested in the last step.",
          label: "Ask the agent",
        },
        title: "Reactivity",
        body: "Now ask the agent to add 3 items to the TODO list, and to set Triggers that fire when you check each one off yourself. The agent adds the items and the Triggers; you tick the boxes. Watch the Triggers react, and the cards move on the board, as you do.",
      }),
      {
        id: "clean-up",
        ask: {
          text: "Remove the onboarding worker from this project.",
          label: "Ask the agent",
        },
        title: "Clean up",
        body: "Removing a worker is the same move as adding one. Ask the agent to take the tour back out, and this card goes with it.",
        // No condition, and this is the one step where that is not a choice:
        // the worker this page waits on is the worker being removed, so
        // nothing survives to report the step done. Sending the prompt IS the
        // step, which is what an `ask` with no condition already does.
      },
    ],
  },
];

/** Tours in curriculum order, each with its step count instead of its steps. */
export function listTours() {
  return TOURS.map((tour) => ({
    id: tour.id,
    title: tour.title,
    description: tour.description,
    step_count: tour.steps.length,
  }));
}

/** @returns {Tour | undefined} */
export function getTour(id) {
  return TOURS.find((tour) => tour.id === id);
}

/**
 * The step, its tour, and its 1-based number in that tour — the number is
 * what the `onboarding:steps:complete` event reports, so it is read from
 * the ordered step list rather than stored beside it. The intro is step 0.
 *
 * @returns {{ tour: Tour, step: Step | Intro, number: number } | undefined}
 */
export function findStep(tourId, stepId) {
  const tour = getTour(tourId);
  if (!tour) return undefined;
  if (stepId === tour.intro.id) return { tour, step: tour.intro, number: 0 };
  const index = tour.steps.findIndex((step) => step.id === stepId);
  if (index < 0) return undefined;
  return { tour, step: tour.steps[index], number: index + 1 };
}
