(function exposeHarnessExecutionTranscript(root, factory) {
  const api = factory();
  if (typeof module === "object" && module.exports) {
    module.exports = api;
  } else {
    root.HarnessExecutionTranscript = api;
  }
})(typeof globalThis === "object" ? globalThis : this, function buildApi() {
  "use strict";

  function contentBlocks(content) {
    if (typeof content === "string") {
      return content.trim() ? [content] : [];
    }
    if (!Array.isArray(content)) return [];
    return content
      .filter((block) => block?.type === "text" && typeof block.text === "string")
      .map((block) => block.text)
      .filter((text) => text.trim());
  }

  function messageText(content) {
    return contentBlocks(content).join("\n\n");
  }

  function eventId(entry, fallback) {
    return String(entry?.entry_id || fallback).replace(/[^a-zA-Z0-9_-]/g, "-");
  }

  function displayFunctionId(functionId, argumentsValue) {
    const wrappedFunction =
      functionId === "agent_trigger" &&
      argumentsValue &&
      typeof argumentsValue === "object" &&
      typeof argumentsValue.function === "string"
        ? argumentsValue.function.trim()
        : "";
    return wrappedFunction || functionId || "unknown function";
  }

  function displayFunctionArguments(functionId, argumentsValue) {
    if (
      functionId === "agent_trigger" &&
      argumentsValue &&
      typeof argumentsValue === "object"
    ) {
      return argumentsValue.payload ?? null;
    }
    return argumentsValue ?? null;
  }

  function normalizeTranscript(messages) {
    const events = [];
    const calls = new Map();

    (Array.isArray(messages) ? messages : []).forEach((entry, entryIndex) => {
      const message = entry?.message;
      const role = message?.role;
      const baseId = eventId(entry, `entry-${entryIndex + 1}`);

      if (role === "user" || role === "assistant") {
        const text = messageText(message.content);
        if (text) {
          events.push({
            id: `${baseId}-message`,
            kind: "message",
            role,
            text,
            timestamp: message.timestamp || null,
            model: message.model || null,
            provider: message.provider || null,
          });
        }

        if (role === "assistant" && Array.isArray(message.content)) {
          message.content.forEach((block, blockIndex) => {
            if (block?.type !== "function_call") return;
            const callId = String(block.id || `${baseId}-call-${blockIndex + 1}`);
            const event = {
              id: eventId(
                { entry_id: `${baseId}-${callId}` },
                `${baseId}-call-${blockIndex + 1}`,
              ),
              kind: "tool",
              callId,
              functionId: displayFunctionId(block.function_id, block.arguments),
              arguments: displayFunctionArguments(block.function_id, block.arguments),
              timestamp: message.timestamp || null,
              result: null,
              isError: false,
              status: "pending",
            };
            events.push(event);
            calls.set(callId, event);
          });
        }
        return;
      }

      if (role === "function_result") {
        const callId = String(message.function_call_id || "");
        const result = {
          text: messageText(message.content),
          details: message.details ?? null,
          timestamp: message.timestamp || null,
        };
        const isError = message.is_error === true;
        const existing = callId ? calls.get(callId) : null;
        if (existing) {
          existing.result = result;
          existing.isError = isError;
          existing.status = isError ? "error" : "completed";
          if (
            existing.functionId === "unknown function" &&
            typeof message.function_id === "string"
          ) {
            existing.functionId = message.function_id;
          }
          return;
        }

        events.push({
          id: `${baseId}-result`,
          kind: "tool",
          callId: callId || null,
          functionId: message.function_id || "unknown function",
          arguments: null,
          timestamp: message.timestamp || null,
          result,
          isError,
          status: isError ? "error" : "completed",
        });
        return;
      }

      if (entry?.custom?.type === "function_call") {
        const status = String(entry.custom.status || "completed").toLowerCase();
        events.push({
          id: `${baseId}-custom`,
          kind: "tool",
          callId: null,
          functionId: entry.custom.name || "unknown function",
          arguments: entry.custom.arguments ?? null,
          timestamp: entry.custom.timestamp || null,
          result: entry.custom.result
            ? { text: "", details: entry.custom.result, timestamp: null }
            : null,
          isError: status === "failed" || status === "error",
          status,
        });
      }
    });

    return events;
  }

  function summarizeTranscript(events) {
    const normalized = Array.isArray(events) ? events : [];
    return normalized.reduce(
      (summary, event) => {
        if (event.kind === "message") summary.messages += 1;
        if (event.kind === "tool") {
          summary.calls += 1;
          if (event.isError) summary.errors += 1;
        }
        return summary;
      },
      { messages: 0, calls: 0, errors: 0 },
    );
  }

  function filterTranscript(events, filter) {
    const normalized = Array.isArray(events) ? events : [];
    if (filter === "messages") {
      return normalized.filter((event) => event.kind === "message");
    }
    if (filter === "errors") {
      return normalized.filter((event) => event.kind === "tool" && event.isError);
    }
    return normalized;
  }

  return {
    contentBlocks,
    displayFunctionArguments,
    displayFunctionId,
    filterTranscript,
    messageText,
    normalizeTranscript,
    summarizeTranscript,
  };
});
