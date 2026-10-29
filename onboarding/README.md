# onboarding

The guided console tour. The page and its tab are called **learn-iii**; a
"tour" is one unit of content inside it, so a second tour can be added
without renaming anything.

Three parts:

| Part      | Lives in                                  | Owns                                                                          |
| --------- | ----------------------------------------- | ----------------------------------------------------------------------------- |
| Content   | `src/tours.mjs`                           | which tours exist, their steps, and the console element each step points at   |
| Progress  | the `state` worker, scope `learn-iii`     | how far each operator got, so a reload resumes mid-tour                       |
| Rendering | `ui/page.tsx` (injected into the console) | the tour card, the spotlight box, and the page an older console falls back to |

## Functions

- `learn-iii::tours::list` — every tour in curriculum order.
- `learn-iii::tours::get` — one tour with all of its steps.
- `learn-iii::progress::get` — the status of every step per tour, with the observed event that closed it, plus the next tour to offer.
- `learn-iii::steps::complete` — mark one step complete. Pass `fired` (`{ function_id, payload }`) when an observed event closed it; the observed function and the event are kept as evidence. The first time a step closes, it is also announced on the `onboarding:steps:complete` topic (see Events).
- `learn-iii::steps::reset` — forget one tour and start it again.
- `learn-iii::subscribe` — add an email address to the iii product-update list (the last step's signup box; the POST happens here, never in the browser).

A step that waits on something carries a `condition`: an observe source the
page observes from the browser with a per-tab target. The agent-finished steps
observe `state::on-changed` with filters on `input.scope` (`learn-iii`) and
`input.key` (`step_<id>_completed`); the first step observes
`harness::on-turn-completed` and counts only turns of the tour's own chat. The
page also observes `console::workspace::on-changed` to follow the layout, and
the worker serves its page through one `console::ui::asset` observer per asset
(`{ kind: 'script' | 'style', path }`).

```js
host.iii.observe(
  {
    function_id: 'state::on-changed',
    filters: { 'input.scope': { $eq: 'learn-iii' }, 'input.key': { $eq: 'step_reactivity_completed' } },
  },
  `learn-iii::condition::learn-iii::reactivity::${host.iii.browserId}`,
)
```

Progress is written with `state::update` and its ordered atomic ops, never
`state::get` then `state::set`: two invocations can interleave around an
await, and a read-then-replace write would drop the earlier one. `merge`
walks and creates the path it names, so one step lands under its own tour.
A reset nulls its tour (the op set reaches top-level keys only for `remove`)
and the read drops nulls on the way out.

## Events

Each closed step is published to the `onboarding:steps:complete` topic through the
`queue` worker, so anything that follows a tour subscribes instead of reading
this worker's state. The payload:

```json
{
  "tour_id": "learn-iii",
  "tour_title": "Find your way around",
  "step_number": 3,
  "step_id": "coder",
  "step_title": "CODER"
}
```

The step number is the 1-based position in the tour, read from the ordered
step list. A step announces itself once per subject: a reload, or a second
report from the agent, writes the same progress again but publishes nothing.

Delivery is durable (`iii::durable::publish`, and subscribers observe
`queue::on-message` with args `{ topic: 'onboarding:steps:complete' }`): the message waits in the queue and is
retried until a subscriber takes it, instead of being dropped when nothing is
listening at that instant. The publish itself is best effort — a project
without the `queue` worker logs the failure and still closes the step.
