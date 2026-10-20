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
const STATE_SCOPE = "learn-iii";

/**
 * The agent profile the tour talks to: the one the harness template ships
 * for its setup examples (`agents/ade-worker-builder.md`, shown as
 * "Onboarding"). The first step starts a new chat under it; every later
 * prompt goes to the chat the operator is in, which is that one unless they
 * moved.
 */
export const TOUR_AGENT = "ade-worker-builder";

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
  const key = `step_${step.id}_completed`;
  return {
    ...step,
    ask: {
      ...step.ask,
      text: `${step.ask.text} When you are ending your turn, write ${key}: true to state, scope "${STATE_SCOPE}".`,
    },
    condition: {
      type: "state",
      config: { scope: STATE_SCOPE, key },
      label: "Waiting for the agent to report this step done",
      hint: `The agent writes ${key} into the "${STATE_SCOPE}" state scope when it has finished.`,
    },
  };
};

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
      body: "Want to learn more about iii, the engine that powers this harness and everything you build? Take a 5 minute tour to learn how iii does things differently.",
    },
    steps: [
      {
        id: "message",
        ask: {
          text: "hey",
          label: "Send a message",
          agent: TOUR_AGENT,
        },
        title: "Send a message",
        body: "Say 'hey' to the agent. The button opens a new chat and sets a 'trigger' here which is how iii handles events and reactivity universally. When the agent fires the trigger this worker will react immediately and continue to the next step. This is not a frontend, backend, data, or other type of trigger, it is universal and works everywhere across a iii application.",
        anchors: [".learn-iii-composer", ".composer-shell"],
        condition: {
          type: "harness::turn-completed",
          config: {},
          label: "Waiting for a harness turn to finish",
          hint: "Say hey in the chat pane.",
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
        body: "The goal of iii is to be composable, observable, discoverable, extensible, and reactive. Let's take a look at each property one by one.",
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
        body: "iii is composable like Node or Python, add libraries and other parts of your application can use them. Except when you add to iii you add a fully working service.\n\nWe call these services 'workers'. When one is added every other worker immediately has access to its functions and triggers.\n\nThis means, for example, that your agent will be able to use the kanban worker without skills, or a long discovery and research process. Now ask the agent to add a kanban board.",
      }),
      {
        id: "stay-in-touch",
        title: "Stay in touch",
        body: "iii moves quickly. Add your email for occasional product updates and tips on making the most out of iii.",
        // No anchor: this step is about the page itself, and there is nothing
        // in the console to point at. The page renders the signup box and the
        // links for this step id.
      },
      {
        id: "observability",
        title: "Observability",
        body: "Everything in a iii worker is defined as either a function or a trigger. This property makes it so that all of a iii application is inherently observable, out of the box, truly end-to-end, even the new worker you're building today. No agent, database, or other usecase-specific framework necessary.\n\nThe observability worker lets you see traces in the UI, query traces directly, ask agents for specific details of what happened, and export them to any OpenTelemetry-compatible service.",
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
          elsewhere:
            "Traces is open in another tab. Switch to that tab, lit up in the tab row, to see it.",
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
          text: "What can this system do right now? List the workers that are registered and the functions each one exposes. Use one unfiltered functions list for the whole answer, not one call per worker.",
          label: "Ask the agent",
        },
        title: "Discoverability",
        body: "An earlier step mentioned that agents don't have a long discoverability step. This project is made up of about 14 or so services. If you ask the agent to report them all you'll get a quick response with the large majority of the time spent waiting on the LLM to generate its text.\n\nHow long would it take your other agents to do this? How accurate would their results be?",
      }),
      agentStep({
        id: "extensibility",
        ask: {
          text: "Build a TODO worker on the todo-local kanban board, the way the onboarding skill says (directory::skills::get { id: \"onboarding\" }, section 'Build the TODO worker'): first set the board to two columns, TODO and Done; then a worker whose console page is a narrow sidebar of controls, not a list: an add form and an edit form with a due date, a due time, a priority, a repeat and an alarm per item, each shown as a pill on the board card. The board is the list and Done is set on the board; a done item with a repeat goes back to TODO with the next date, the same ticket, never a second card; alarms go through the cron worker; a todo:changed trigger type; and the same controls as a web page leveraging the iii browser sdk. Verify through functions, not screenshots, and do not navigate me away from this workspace. When it is done, open the TODO page to the left of the board as a narrow column (pass sizes so it takes the console's smallest column width, 12%), and open the web page for me in the console's browser in a new tab (browser::sessions::start with preview: false, then console::workspace::open { screen: 'browser', placement: 'new-tab', activate: false }), not the preview window, and leave me on the tab with the TODO page and the board: do not switch me to the browser tab.",
          label: "Ask the agent",
        },
        title: "Extensibility",
        body: "Let's extend this application with a new worker, and while doing so we'll take advantage of iii's composability by using the existing Kanban, and Cron workers for the bulk of the work.\n\nWe'll build a simple TODO sidebar of controls that can handle specific properties. In the future it's likely we'll want to leverage the Kanban board for other projects, so we'll put our specific needs in their own worker.\n\nWe'll add a due date, a due time, a priority, a repeat and an alarm (via Cron) on each item. We'll also add the same controls as a web page.\n\nThe web page itself is also a worker just like anything else in iii. So it can make direct calls to other workers, without the need for a REST API in between.\n\nWe won't add it now but any worker can also be gated with role based access control so untrusted clients like websites can only do what they're allowed to.",
        // The step closes when the agent reports done; the panel goes as the
        // step opens, so the new page has room.
        to_close: { screen: "traces" },
      }),
      agentStep({
        id: "reactivity",
        ask: {
          text: "Add 3 items to the TODO list, each with a due date and a priority, and set a Trigger that fires when each one is moved to Done on the board by the user, not you the agent. Pre-existing triggers should work, and were tested in the last step.",
          label: "Ask the agent",
        },
        title: "Reactivity",
        body: "Now ask the agent to add 3 items to the TODO list, and to set triggers that fire when you move each one to Done yourself. Watch the agent react as you change the card's status.\n\nLike mentioned earlier this isn't frontend, backend, or data reactivity, this is universal reactivity. Everything is both immediately reactive and immediately able to react to anything that happens in a iii system.",
      }),
      {
        id: "clean-up",
        ask: {
          text: "Remove the onboarding worker from this project.",
          label: "Ask the agent",
        },
        title: "Clean up",
        body: "Removing a worker is the same move as adding one. Ask the agent to remove this worker and this card will close with it.",
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
