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
 *   - `ask` (optional): a prompt the step's button puts in the chat composer
 *     and sends, so the operator talks to the agent instead of copying text
 *     out of the tour.
 *   - `on_closed` (optional): a console screen the step suggests closing, and
 *     the body and anchor to show once it is gone. The page watches the
 *     console's own configuration entry — the workspace layout lives there,
 *     so a closed pane IS a configuration write and the `configuration`
 *     trigger reports it. Never required to finish the step.
 *   - `condition` (optional): a real engine trigger the step waits for. The
 *     page binds it, shows what it is waiting for, and when it fires shows
 *     the trigger and its payload. A step with no condition is closed by the
 *     operator with a button.
 */

/**
 * @typedef {{ type: string, config: Record<string, unknown>, label: string, hint?: string, prompt?: string }} Condition
 * @typedef {{ text: string, label: string }} Ask
 * @typedef {{ screen: string, body: string, anchors?: string[] }} OnClosed
 * @typedef {{ id: string, title: string, body: string, anchors?: string[], condition?: Condition, screen?: string, ask?: Ask, on_closed?: OnClosed }} Step
 * @typedef {{ id: string, title: string, description: string, steps: Step[] }} Tour
 */

/**
 * The state scope the tour keeps everything in — progress, and the keys the
 * steps below wait on.
 */
const STATE_SCOPE = 'onboarding'

/**
 * A step the AGENT finishes, not the operator.
 *
 * Its prompt ends by telling the agent to write `step_<id>_completed` into
 * the onboarding state scope, and the step waits on a `state` trigger bound
 * to that one key. The page binds every reachable condition when the step is
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
      text: `${step.ask.text} When you are done, write ${key}: true to state, scope "${STATE_SCOPE}".`,
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
          label: "Say hi",
        },
        title: "Send a message",
        body: "Say 'hello' to the agent. We've setup a trigger to run when you get the agent's response. The iii ade (agentic development environment) is an example of a iii Worker. Database and state are other examples of workers. Every Worker in the iii system can be composed, observed, and reacted to in the same way.",
        anchors: [".onboarding-composer", ".composer-shell"],
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
        body: "Each tab is a workspace of one or more panes, side by side. You can open as many or as you need.",
        anchors: [
          ".onboarding-tabs",
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
        id: "observability",
        title: "Observability",
        body: "Open Traces to see the message you just sent. Each function call and each trigger writes a span, so a trace shows which worker ran, in which order, and how long each part took. The iii-observability worker collects the spans and can export them to any OpenTelemetry backend.",
        anchors: [".onboarding-traces", 'section[aria-label="traces"]'],
        // The step's button places this console screen beside the tour, so the
        // operator reads the trace instead of hunting for the palette row.
        screen: "traces",
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
          text: "Using the database worker, build me a TODO list: a CRUD app with an injectable console UI on the browser SDK. Make it reactive with triggers: a database::row-changed trigger on the todo table. Open the page for me when it is done.",
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
          // The console publishes no `onboarding-*` class on its new-tab
          // button, so this rides the button's own stable aria-label.
          anchors: ['[aria-label="New workspace"]'],
        },
      }),
      agentStep({
        id: "reactivity",
        ask: {
          text: "Add 3 items to the TODO list, and set a Trigger that fires when each one is checked off.",
          label: "Ask the agent",
        },
        title: "Reactivity",
        body: "Now ask the agent to add 3 items to the TODO list, and to set Triggers to listen for when they're done. Watch the Triggers react as you check the boxes.",
      }),
      {
        id: "stay-in-touch",
        title: "Stay in touch",
        body: "iii moves quickly. Put your email in for the roadmap and the product updates — what is being built, and what shipped. The links below go to the same places, if you would rather read along there.",
        // No anchor: this step is about the page itself, and there is nothing
        // in the console to point at. The page renders the signup box and the
        // links for this step id.
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

/** @returns {Step | undefined} */
export function getStep(tourId, stepId) {
  return getTour(tourId)?.steps.find((step) => step.id === stepId);
}
