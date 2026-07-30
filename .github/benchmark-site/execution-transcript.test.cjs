const test = require("node:test");
const assert = require("node:assert/strict");

const {
  displayFunctionId,
  filterTranscript,
  messageText,
  normalizeTranscript,
  summarizeTranscript,
} = require("./execution-transcript.js");

test("shows the wrapped function name instead of agent_trigger", () => {
  assert.equal(
    displayFunctionId("agent_trigger", {
      function: "database::query",
      payload: { db: "primary" },
    }),
    "database::query",
  );
  assert.equal(displayFunctionId("state::get", { scope: "test" }), "state::get");
  assert.equal(displayFunctionId("agent_trigger", { payload: {} }), "agent_trigger");
});

test("extracts readable text from current and legacy message content", () => {
  assert.equal(messageText(" Legacy response. "), " Legacy response. ");
  assert.equal(
    messageText([
      { type: "text", text: "First" },
      { type: "function_call", function_id: "state::get" },
      { type: "text", text: "Second" },
    ]),
    "First\n\nSecond",
  );
});

test("pairs function results with calls and marks recovered errors", () => {
  const events = normalizeTranscript([
    {
      entry_id: "user-1",
      message: {
        role: "user",
        content: [{ type: "text", text: "Verify the state." }],
      },
    },
    {
      entry_id: "assistant-1",
      message: {
        role: "assistant",
        timestamp: "2026-07-30T12:00:00Z",
        content: [
          { type: "text", text: "I will inspect it." },
          {
            type: "function_call",
            id: "call-ok",
            function_id: "agent_trigger",
            arguments: {
              function: "state::get",
              payload: { key: "status" },
            },
          },
          {
            type: "function_call",
            id: "call-error",
            function_id: "shell::exec",
            arguments: { command: "false" },
          },
        ],
      },
    },
    {
      entry_id: "result-error",
      message: {
        role: "function_result",
        function_call_id: "call-error",
        function_id: "shell::exec",
        is_error: true,
        details: { error: "command failed" },
        content: [{ type: "text", text: "command failed" }],
      },
    },
    {
      entry_id: "result-ok",
      message: {
        role: "function_result",
        function_call_id: "call-ok",
        function_id: "state::get",
        is_error: false,
        details: { value: "ready" },
        content: [{ type: "text", text: "{\"value\":\"ready\"}" }],
      },
    },
  ]);

  assert.deepEqual(
    events.map((event) => [event.kind, event.functionId || event.role]),
    [
      ["message", "user"],
      ["message", "assistant"],
      ["tool", "state::get"],
      ["tool", "shell::exec"],
    ],
  );
  assert.equal(events[2].result.details.value, "ready");
  assert.equal(events[2].status, "completed");
  assert.equal(events[3].isError, true);
  assert.equal(events[3].result.details.error, "command failed");
});

test("summarizes and filters a conversation for fast error inspection", () => {
  const events = [
    { kind: "message", role: "user" },
    { kind: "message", role: "assistant" },
    { kind: "tool", isError: false },
    { kind: "tool", isError: true },
  ];

  assert.deepEqual(summarizeTranscript(events), {
    messages: 2,
    calls: 2,
    errors: 1,
  });
  assert.equal(filterTranscript(events, "messages").length, 2);
  assert.equal(filterTranscript(events, "errors").length, 1);
  assert.equal(filterTranscript(events, "all").length, 4);
});
