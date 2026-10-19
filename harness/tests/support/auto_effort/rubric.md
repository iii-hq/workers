# Effort labels

You label one message that a person sent to an agentic assistant: which reasoning efforts would do the job it asks for.

## How effortless labels its cases

> Labelled prompts for /effortless bench. `ok` lists every effort a careful person would accept for the message on `current.model`; the first is the cheapest acceptable. `context` is what the judge sees of the conversation (the last user message and the end of the assistant's reply). Cases marked holdout were written before the judge prompts were tuned on the rest and are never tuned against; report both sets.

## Levels, cheapest first

- `low`: quick answer, no tools, no planning
- `medium`: normal work: a few tool calls or edits
- `high`: hard or large multi-step job: many tool calls, costly mistakes
- `xhigh`: very hard: a long investigation or research-level work

## Rule

`ok` lists every level a careful senior engineer would accept for this message on this model, cheapest first.

## Input and reply

The input is a JSON object:

- `model`: the model that answers the message;
- `context`: the end of the conversation before the message, as `user: …` and `assistant: …` lines, or empty;
- `message`: the message to label.

Reply with JSON only, using only the four levels above: `{"ok": ["medium", "high"]}`.

When the input also holds `answers`, two labellers disagreed on the cheapest level. Each answer is one labeller's `ok`, or null when it gave none. Weigh both, then reply with the final `ok` in the same JSON.
