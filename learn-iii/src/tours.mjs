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
 *     out of the tour.
 *   - `on_closed` (optional): a console screen the step suggests closing, and
 *     the body and anchor to show once it is gone. The page binds the
 *     console's `console::workspace::changed` trigger, which rings after
 *     every layout write, so a closed pane is reported as it happens. Never
 *     required to finish the step.
 *   - `to_open` (optional): a console screen the step talks about and the
 *     operator opens THEMSELVES. While it is on screen the step frames its
 *     own anchors; while it is not, the page shows this body and frames
 *     these anchors instead — where the screen opens — and the step's
 *     button waits until it is there. Open only in a tab that is not
 *     showing, the page shows `elsewhere` and frames that tab. Watched the
 *     same way as `on_closed`.
 *   - `condition` (optional): a real engine trigger the step waits for. The
 *     page binds it, shows what it is waiting for, and when it fires shows
 *     the trigger and its payload. A step with no condition is closed by the
 *     operator with a button.
 */

/**
 * @typedef {{ type: string, config: Record<string, unknown>, label: string, hint?: string, prompt?: string }} Condition
 * @typedef {{ text: string, label: string }} Ask
 * @typedef {{ screen: string, body: string, anchors?: string[] }} OnClosed
 * @typedef {{ screen: string, body: string, anchors: string[], elsewhere?: string }} ToOpen
 * @typedef {{ id: string, title: string, body: string, anchors?: string[], condition?: Condition, ask?: Ask, on_closed?: OnClosed, to_open?: ToOpen }} Step
 * @typedef {{ id: string, title: string, description: string, steps: Step[] }} Tour
 */

/**
 * The state scope the tour keeps everything in — progress, and the keys the
 * steps below wait on.
 */
const STATE_SCOPE = 'learn-iii'

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
    id: "console-basics",
    title: "Find your way around",
    description: "The surfaces of the console, and the engine underneath them.",
    steps: [
      {
        id: "message",
        ask: {
          text: "hello",
          label: "Say hello",
        },
        title: "Send a message",
        body: "Say 'hello' to the agent. We've setup a trigger to run when you get the agent's response. The iii ade (agentic development environment) is an example of a iii Worker. Database and state are other examples of workers. Every Worker in the iii system can be composed, observed, and reacted to in the same way.",
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
          text: "Add a database worker to this project, then tell me what it can do.",
          label: "Ask the agent",
        },
        title: "Composability",
        body: "iii is composable like Node or Python, except when you add to iii you're adding a working service and not a library. Let's ask the agent to add a database worker.",
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
        body: "Traces follows the chat you are in and opens its latest turn. Each function call and each trigger writes a span, so a trace shows which worker ran, in which order, and how long each part took. The iii-observability worker collects the spans and can export them to any OpenTelemetry backend.",
        anchors: [".learn-iii-traces", 'section[aria-label="traces"]'],
        // Opened by the operator, not for them: a fresh workspace already has
        // Traces beside the chat, and when it is gone the page points at where
        // it opens — the + first, then the traces row of the new tab's picker
        // (best match first, so the box moves on once the picker is up). A
        // phone has no tab row: its search button is how Traces opens there.
        to_open: {
          screen: "traces",
          body: "Traces is not open. Open a new tab and pick traces from its list, or search for traces.",
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
          text: "Using the database worker, build me a TODO list: a CRUD app with an injectable console UI on the browser SDK. Make it reactive with triggers: a database::row-changed trigger on the todo table. Ensure the TODO application is reactive to triggers (ie. database changes) and that emits Triggers when a change happens (ie. register trigger type). Do not navigate the user away from the current workspace. Do not use screenshots or other UI based checks. Verify via functions. This is an learn-iii demo. Open the TODO page for me when it is done.",
          label: "Ask the agent",
        },
        title: "Extensibility",
        body: "Great! Now let's use that database worker. Ask the agent to create a simple CRUD app like a TODO list and to create console injectable UI for it as well and to open it for you when it's done. Since we're going to build and open our very own worker we can make room by closing the Traces panel (optional).",
        // The close is an invitation, not a requirement: the step closes
        // when the agent reports done. When the operator does take it, the
        // page swaps in this note so dropping the panel is never a dead end.
        on_closed: {
          screen: "traces",
          body: "You can always open traces in a new tab.",
          // The console publishes no `learn-iii-*` class on its new-tab
          // button, so this rides the button's own stable aria-label.
          anchors: ['[aria-label="New workspace"]'],
        },
      }),
      agentStep({
        id: "reactivity",
        ask: {
          text: "Add 3 items to the TODO list, and set a Trigger that fires when each one is checked off by the user, not you the agent. Pre-existing triggers should work, and were tested in the last step.",
          label: "Ask the agent",
        },
        title: "Reactivity",
        body: "Now ask the agent to add 3 items to the TODO list, and to set Triggers that fire when you check each one off yourself. The agent adds the items and the Triggers; you tick the boxes. Watch the Triggers react as you do.",
      }),
      {
        id: "clean-up",
        ask: {
          text: "Remove the learn-iii worker from this project.",
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
 * what the `learn-iii:steps:complete` event reports, so it is read from
 * the ordered step list rather than stored beside it.
 *
 * @returns {{ tour: Tour, step: Step, number: number } | undefined}
 */
export function findStep(tourId, stepId) {
  const tour = getTour(tourId);
  const index = tour?.steps.findIndex((step) => step.id === stepId) ?? -1;
  if (!tour || index < 0) return undefined;
  return { tour, step: tour.steps[index], number: index + 1 };
}

/**
 * A step's condition matches an event when every key its config filters on
 * has the same value in the payload; an empty config matches any event of
 * the type, the way the engine applies the binding.
 */
export function conditionMatches(condition, triggerType, payload) {
  return (
    condition?.type === triggerType &&
    Object.entries(condition.config ?? {}).every(([key, value]) => payload?.[key] === value)
  );
}
