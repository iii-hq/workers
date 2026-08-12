//! The JSON envelope every JS↔Rust value crosses in.
//!
//! Nothing depends on `serde_v8` conversion semantics: JavaScript stringifies
//! its own values with `JSON.stringify` and Rust parses the resulting string,
//! so a value means the same thing on both sides or it is not representable.

use serde_json::Value;

use crate::error::NodeEngineError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Envelope {
    Ok(Value),
    Err(String),
}

impl Envelope {
    pub fn parse(s: &str) -> Result<Envelope, NodeEngineError> {
        let v: Value = serde_json::from_str(s).map_err(|e| {
            NodeEngineError::eval_failed(format!("runtime returned a non-JSON envelope: {e}"))
        })?;
        if let Some(err) = v.get("err") {
            return Ok(Envelope::Err(
                err.as_str().unwrap_or("unknown error").to_string(),
            ));
        }
        match v.get("ok") {
            Some(ok) => Ok(Envelope::Ok(ok.clone())),
            None => Err(NodeEngineError::eval_failed(format!(
                "runtime returned an envelope with neither ok nor err: {s}"
            ))),
        }
    }
}

/// JSON string literals are valid JavaScript string literals, so serde_json is
/// the escaper for anything interpolated into generated source.
fn js_string(s: &str) -> String {
    serde_json::to_string(s).expect("string serializes")
}

/// Wrap caller code in an async IIFE so top-level `await` works, and hand the
/// resulting promise to the prelude's `settle` so it comes back as an envelope.
pub fn wrap_eval(code: &str) -> String {
    format!("globalThis.__iii.settle((async () => {{\n{code}\n}})())")
}

/// Source that invokes a registered JS handler, or — when `method` is given —
/// one of a registered trigger TYPE's two callback methods. `fn_id`/`type_id`
/// and the payload are escaped as JS string literals; the payload crosses as
/// JSON text the prelude parses. `method` reaches `__iii.invoke` as a bare
/// JS argument (a quoted string literal, or the literal token `undefined`)
/// rather than JSON text, matching how the prelude's `invoke(id, payloadJson,
/// method)` reads it directly with no `JSON.parse`.
pub fn wrap_invoke(fn_id: &str, payload: &Value, method: Option<&str>) -> String {
    let payload_json = serde_json::to_string(payload).expect("payload serializes");
    let method_arg = match method {
        Some(m) => js_string(m),
        None => "undefined".to_string(),
    };
    format!(
        "globalThis.__iii.settle(globalThis.__iii.invoke({}, {}, {}))",
        js_string(fn_id),
        js_string(&payload_json),
        method_arg
    )
}

/// Source that resolves `source` to a handler function, registers it, then
/// reports the form it resolved to.
///
/// This is a STATEMENT BODY for [`wrap_eval`], not a standalone expression:
/// `wrap_eval` already supplies the async IIFE and `settle`, so a throw here
/// reaches the caller as `eval_failed`. Wrapping this in its own IIFE would
/// detach the promise and report success on every failure. The trailing
/// `return` is what `wrap_eval`'s IIFE turns into the eval's `result`.
///
/// `source` must DEFINE `handler(payload)` — the documented shape is
/// `export function handler(p) { ... }`, and `const`/`let handler = ...`
/// are equally valid ways to define it. (`class handler {}` satisfies the
/// generated wrapper's `typeof handler === "function"` check and registers,
/// but a class constructor has no `[[Call]]`, so every invocation then
/// throws — see `a_class_named_handler_registers_but_can_never_be_invoked`.
/// It is not a supported form and no document recommends it.) `__iii.
/// toHandler` (prelude.js) still only resolves ONE JavaScript *expression*
/// to a function — that mechanism, and the error mapping built on it (no
/// internal stack), is unchanged and worth keeping exactly as
/// `register_function`'s old batch loop left it. So `source` is turned into
/// an expression here, in Rust, rather than by touching prelude.js: a
/// leading `export` is stripped (deno_core runs this as a plain script, and
/// `export` is a SyntaxError anywhere outside a module — see the runtime
/// comment on `execute_script`), and the whole thing is wrapped so the
/// expression resolves to whatever `source` bound `handler` to, or throws a
/// clean error if it bound nothing (or something that is not a function).
///
/// The wrapper is two nested IIFEs, not one — this shape, and every claim
/// below, was checked against real V8 (`node -e`, deno_core embeds the same
/// engine), not derived by reasoning about the spec alone:
///
/// ```js
/// (function(){var handler;
/// handler=(function(){
/// <stripped source>
/// return typeof handler==="function"?handler:undefined;
/// })();
/// if(typeof handler!=="function")throw new TypeError("source must define handler(payload)");
/// return handler;
/// })()
/// ```
///
/// - **The inner IIFE, not `var handler;` alone, is what makes `const`/
///   `let`/`class handler` legal.** An earlier version of this wrapper put
///   `var handler;` directly ahead of `<stripped source>` in ONE scope —
///   which works for `function handler(){}` and a bare `handler = ...`
///   assignment, but a `var` and a `let`/`const`/`class` of the SAME name in
///   the SAME scope is a `SyntaxError` ("Identifier 'handler' has already
///   been declared"), and `export const handler = async (p) => ...` is the
///   canonical ESM idiom — exactly what "defining `handler(payload)`"
///   invites an agent to write. Giving `<stripped source>` its OWN function
///   scope (the inner IIFE) means whatever declaration form it uses for
///   `handler` never collides with the OUTER `var handler;`, which exists
///   purely to receive the inner IIFE's result.
/// - **`typeof handler === "function"` in the OUTER scope, not a bare
///   `return handler` inside the inner one** — `typeof` is the one JS
///   operator that never throws on an unresolvable reference, so a `source`
///   that binds nothing fails this check with a message this function
///   writes, rather than raising a `ReferenceError` that `toHandler`'s own
///   handling (built for the OLD, now-retired bare-expression contract)
///   would catch and answer with a hint that quotes this whole generated
///   wrapper — newlines included — back at the caller.
/// - **The outer `var handler;` still matters** even with the source in its
///   own scope: the namespace runtime is ONE isolate shared by every
///   registration into it, and `new Function` runs in sloppy mode — a
///   PREVIOUS registration's `handler = ...` (undeclared, hence a global in
///   ITS inner IIFE, same as any sloppy-mode top-level assignment) must not
///   be visible to a LATER `source` that binds nothing. The outer `var
///   handler;` shadows any such stale global before the outer `typeof`
///   check ever runs, so "binds nothing" always means "sees its own
///   `undefined`", never "inherits a neighbour's leftover function".
/// - **The inner IIFE's `return` is what makes the "must define handler"
///   check unconditional.** A `source` containing its own top-level
///   `return` (mimicking the OLD body-form contract) still only returns
///   from the INNER function; the outer `typeof handler !== "function"`
///   check runs regardless, so a non-function early return (`"return 42"`)
///   is still refused — closing that case of the deferred "check is
///   bypassable" finding. (A `source` whose early return happens to BE a
///   function is still accepted, same as before: that is the retired
///   bare-expression contract smuggled back in through `return`, a
///   narrower, still-deferred gap, not something this wrapper newly opens.)
/// - **A `source`'s own `"use strict"` directive prologue survives** as a
///   side effect of the inner IIFE: the prologue must be the first
///   statement(s) of a function body to take effect, and with the source
///   now running as the FIRST content of the inner function (not preceded
///   by `var handler;`, which broke prologue position in the earlier,
///   single-scope version), a tenant that opts into strict mode to catch a
///   typo keeps that protection.
pub fn wrap_register(
    id: &str,
    source: &str,
    description: Option<&str>,
    request_format: Option<&serde_json::Value>,
    response_format: Option<&serde_json::Value>,
) -> String {
    let stripped = strip_leading_export(source);
    let handler_expr = format!(
        "(function(){{var handler;\n\
         handler=(function(){{\n{stripped}\n\
         return typeof handler===\"function\"?handler:undefined;\n\
         }})();\n\
         if(typeof handler!==\"function\")throw new TypeError(\"source must define handler(payload)\");\n\
         return handler;\n\
         }})()"
    );
    // A missing description is an ABSENT property, not `null`: the prelude's
    // third argument is optional and type-checked, so `null` would be a
    // TypeError while `undefined` takes the default.
    let desc = match description {
        Some(text) => format!(",desc:{}", js_string(text)),
        None => String::new(),
    };
    // Formats embed as compact-JSON literals — JSON is a valid JS expression,
    // and the `__def` literal evaluates before any tenant source runs. An
    // absent format leaves its key out, so `__def.reqf` reads `undefined` —
    // "not supplied" — through the same options plumbing as `desc`.
    let reqf = match request_format {
        Some(schema) => format!(",reqf:{schema}"),
        None => String::new(),
    };
    let resf = match response_format {
        Some(schema) => format!(",resf:{schema}"),
        None => String::new(),
    };
    // `registerFunction`'s third argument is now an options OBJECT (the SDK
    // shape — see `prelude.js`), not the raw description string: passing
    // `__def.desc` positionally would hand a string where an object is
    // required and every registration from source would throw. `desc` absent
    // means `__def.desc` reads as `undefined`, so `{description:__def.desc}`
    // still lands on `options.description === undefined` — "not supplied" —
    // exactly like today.
    format!(
        "const __def={{id:{},src:{}{desc}{reqf}{resf}}};\
         const __h=__iii.toHandler(__def.id,__def.src);\
         iii.registerFunction(__def.id,__h.fn,{{description:__def.desc,\
         request_format:__def.reqf,response_format:__def.resf}});\
         return {{id:__def.id,form:__h.form}};",
        js_string(id),
        js_string(&handler_expr)
    )
}

/// Strip a leading `export` keyword — only when it is a real word boundary,
/// and only when what follows is not `default`. Both are cases where
/// dropping just the word `export` would MANGLE the source into something
/// that still fails, but for a different, more confusing reason, rather
/// than leaving it to fail cleanly on the `export` token it actually
/// contains:
/// - `exports.handler = ...` (a plain identifier that happens to start with
///   the same five letters) would become `s.handler = ...` without a
///   boundary check.
/// - `export default function handler(){}` cannot become a bare declaration
///   by dropping only the keyword — `default function handler(){}` is
///   still not valid JavaScript on its own — so it is left untouched too.
fn strip_leading_export(source: &str) -> &str {
    let trimmed = source.trim_start();
    let Some(rest) = trimmed.strip_prefix("export") else {
        return trimmed;
    };
    let Some(boundary) = rest.chars().next() else {
        return trimmed;
    };
    if !boundary.is_whitespace() {
        return trimmed;
    }
    let rest = rest[boundary.len_utf8()..].trim_start();
    if rest.starts_with("default") {
        return trimmed;
    }
    rest
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_ok_envelope() {
        assert_eq!(
            Envelope::parse(r#"{"ok":{"a":1}}"#).unwrap(),
            Envelope::Ok(json!({"a": 1}))
        );
    }

    #[test]
    fn parses_null_ok_envelope() {
        assert_eq!(
            Envelope::parse(r#"{"ok":null}"#).unwrap(),
            Envelope::Ok(Value::Null)
        );
    }

    #[test]
    fn parses_err_envelope() {
        assert_eq!(
            Envelope::parse(r#"{"err":"boom"}"#).unwrap(),
            Envelope::Err("boom".into())
        );
    }

    #[test]
    fn rejects_garbage() {
        assert!(Envelope::parse("not json").is_err());
        assert!(Envelope::parse(r#"{"other":1}"#).is_err());
    }

    #[test]
    fn wrap_eval_embeds_code_in_an_async_iife() {
        let src = wrap_eval("return 1 + 1");
        assert!(src.contains("__iii.settle"));
        assert!(src.contains("return 1 + 1"));
        assert!(src.contains("async ()"));
    }

    /// Code arrives from untrusted callers; the invoke wrapper must never let
    /// a quote or backslash in the function id or payload break out of its
    /// string literal.
    #[test]
    fn wrap_invoke_escapes_its_arguments() {
        let src = wrap_invoke(r#"ns::a"b\c"#, &json!({ "s": "\"quoted\"" }), None);
        assert!(src.contains(r#"ns::a\"b\\c"#));
        assert!(!src.contains("\n"));
    }

    /// A plain function invocation (the pre-Task-12 shape) sends the literal
    /// token `undefined` as the third argument, not a string — the prelude's
    /// `invoke(id, payloadJson, method)` branches on `method` being falsy,
    /// and `"undefined"` (a quoted string) is truthy.
    #[test]
    fn wrap_invoke_sends_the_bare_undefined_token_for_no_method() {
        let src = wrap_invoke("test::x", &json!({}), None);
        assert!(
            src.contains(", undefined)"),
            "method argument must be the bare token, not a quoted string: {src}"
        );
    }

    /// A trigger-type callback carries its method name as a proper JS string
    /// literal, escaped exactly like `fn_id` and the payload.
    #[test]
    fn wrap_invoke_escapes_a_method_name() {
        let src = wrap_invoke("app::mytype", &json!({}), Some(r#"a"b"#));
        assert!(src.contains(r#""a\"b""#), "method not escaped: {src}");
    }

    /// Formats embed as JSON literals in `__def` and ride the options
    /// object; absent formats leave the keys out entirely, so the prelude
    /// reads `undefined` — "not supplied" — exactly like the description.
    #[test]
    fn wrap_register_carries_formats_only_when_given() {
        let schema = serde_json::json!({"type": "object"});
        let with = wrap_register(
            "ns::a",
            "export function handler(p) { return p }",
            None,
            Some(&schema),
            None,
        );
        assert!(with.contains(r#"reqf:{"type":"object"}"#), "{with}");
        assert!(with.contains("request_format:__def.reqf"), "{with}");
        assert!(with.contains("response_format:__def.resf"), "{with}");

        let without = wrap_register(
            "ns::a",
            "export function handler(p) { return p }",
            None,
            None,
            None,
        );
        assert!(!without.contains("reqf:"), "{without}");
        assert!(!without.contains("resf:"), "{without}");
    }

    #[test]
    fn wrap_register_escapes_ids_and_source() {
        let src = wrap_register(
            r#"ns::a"b"#,
            "export function handler(p) {\n  return `x`\n}",
            None,
            None,
            None,
        );
        // Both interpolations are JSON string literals, so nothing can break
        // out of them — including the newline, which would otherwise comment
        // out the rest of the generated script.
        assert!(src.contains(r#"ns::a\"b"#), "id not escaped: {src}");
        assert!(src.contains(r#"return `x`\n"#), "source not escaped: {src}");
        assert!(!src.contains('\n'), "generated source must be one line");
        assert!(
            src.trim_end()
                .ends_with("return {id:__def.id,form:__h.form};"),
            "must end with the return that becomes the eval's result: {src}"
        );
    }

    /// `wrap_eval` already supplies the async IIFE and `settle`. A second IIFE
    /// here would detach the inner promise: the outer would resolve undefined,
    /// the envelope would read `{"ok":null}`, and EVERY failure would be
    /// reported to the caller as success.
    #[test]
    fn wrap_register_is_a_statement_body_not_an_iife() {
        let src = wrap_register(
            "ns::a",
            "export function handler(p) { return p }",
            None,
            None,
            None,
        );
        assert!(
            !src.trim_start().starts_with("(async"),
            "double-wrapped: {src}"
        );
        assert!(src.contains("__iii.toHandler"));
        assert!(src.contains("iii.registerFunction"));
        // The trailing `return` is what makes this a statement body rather
        // than a dead expression: `wrap_eval`'s IIFE turns it into the
        // eval's `result`.
        assert!(
            src.contains("return {id:__def.id,form:__h.form};"),
            "missing the return that becomes the eval's result: {src}"
        );
    }

    /// `source` defines `handler(payload)`, documented as
    /// `export function handler(p) { ... }` — `export` is invalid anywhere
    /// outside a module, including nested inside the generated IIFE, so it
    /// must be stripped before the source is embedded.
    #[test]
    fn wrap_register_strips_a_leading_export() {
        let src = wrap_register(
            "ns::a",
            "export function handler(p) { return 1 }",
            None,
            None,
            None,
        );
        assert!(
            !src.contains("export"),
            "a literal `export` would be a SyntaxError once embedded: {src}"
        );
    }

    /// `exports.handler = ...` is a plain identifier that happens to start
    /// with the same five letters as the keyword — stripping without a word
    /// boundary would mangle it into `s.handler = ...`. `export default
    /// function handler(){}` cannot become a bare declaration by dropping
    /// only the keyword (`default function handler(){}` is still not valid
    /// JavaScript on its own), so it is left untouched too, rather than
    /// mangled into a different, more confusing failure.
    #[test]
    fn strip_leading_export_requires_a_real_word_boundary() {
        assert_eq!(
            strip_leading_export("export function handler(){}"),
            "function handler(){}"
        );
        assert_eq!(
            strip_leading_export("exports.handler = 1"),
            "exports.handler = 1",
            "must not mangle a plain identifier into `s.handler`"
        );
        assert_eq!(
            strip_leading_export("export default function handler(){}"),
            "export default function handler(){}",
            "must not mangle `export default` into a bare `default ...`"
        );
        assert_eq!(
            strip_leading_export("function handler(){}"),
            "function handler(){}"
        );
    }

    /// A description is an ABSENT property when omitted, not `null` — the
    /// prelude's third argument is type-checked and only `undefined` takes
    /// its default.
    #[test]
    fn wrap_register_omits_absent_description() {
        let with = wrap_register(
            "ns::a",
            "export function handler() {}",
            Some("does a thing"),
            None,
            None,
        );
        assert!(with.contains(r#"desc:"does a thing""#), "{with}");
        let without = wrap_register("ns::a", "export function handler() {}", None, None, None);
        assert!(!without.contains("desc:"), "{without}");
    }
}
