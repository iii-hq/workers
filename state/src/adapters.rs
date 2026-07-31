//! Storage adapter implementations for the state worker (e.g. in-memory, file-based, Redis).
//!
//! Trait shape from engine/src/workers/state/adapters/mod.rs; kv wraps the
//! ported KvStore; redis is a port of
//! engine/src/workers/state/adapters/redis_adapter.rs (same `state:<scope>`
//! hash keys, same atomic Lua scripts).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use iii_helpers::stream::{StreamSetResult, StreamUpdateResult, UpdateOp};
use redis::{AsyncCommands, Client, aio::ConnectionManager};
use serde_json::Value;
use tokio::sync::Mutex;
use tokio::time::timeout;

use crate::config::StateConfig;
use crate::store::KvStore;

const DEFAULT_REDIS_URL: &str = "redis://localhost:6379";
const REDIS_CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
const REDIS_CAS_MAX_ATTEMPTS: usize = 8;

pub(crate) fn cas_matches(expected: Option<&Value>, current: Option<&Value>) -> bool {
    match (expected, current) {
        (None | Some(Value::Null), None | Some(Value::Null)) => true,
        (Some(want), Some(got)) => want == got,
        _ => false,
    }
}

#[derive(Debug, PartialEq)]
pub enum CompareAndSetOutcome {
    Swapped { old_value: Option<Value> },
    NotSwapped { current: Value },
}

#[async_trait]
pub trait StateAdapter: Send + Sync + 'static {
    async fn set(&self, scope: &str, key: &str, value: Value) -> anyhow::Result<StreamSetResult>;
    async fn get(&self, scope: &str, key: &str) -> anyhow::Result<Option<Value>>;
    async fn delete(&self, scope: &str, key: &str) -> anyhow::Result<()>;
    async fn update(
        &self,
        scope: &str,
        key: &str,
        ops: Vec<UpdateOp>,
    ) -> anyhow::Result<StreamUpdateResult>;
    /// Swap `scope/key` from `expected` to `value` atomically. `expected:
    /// None` means "expect absent". Returns the observed old value when the
    /// swap happened, or the current value when it did not.
    ///
    /// `get` then `set` from outside cannot express this: two callers reading
    /// the same value both believe they hold it, and both write.
    async fn compare_and_set(
        &self,
        scope: &str,
        key: &str,
        expected: Option<&Value>,
        value: Value,
    ) -> anyhow::Result<CompareAndSetOutcome>;

    /// Apply one barrier arrival atomically to `scope/key`.
    ///
    /// Not expressible with the `UpdateOp` set (there is no compare-and-set),
    /// and it must not be a get-then-set from outside: two arrivals racing on
    /// the last slot would both see "incomplete" and both answer `allow`.
    /// Adapters implement it with whatever atomicity they actually have.
    async fn barrier_arrive(
        &self,
        scope: &str,
        key: &str,
        cfg: &crate::barrier::BarrierConfig,
        event: &Value,
    ) -> anyhow::Result<crate::barrier::Decision>;
    async fn list(&self, scope: &str) -> anyhow::Result<Vec<Value>>;
    /// Keys within a scope, in the adapter's natural order (kv: insertion
    /// order; redis: hash-field order). Added for the console state UI —
    /// `list` returns values only, which cannot drive per-item navigation.
    async fn list_keys(&self, scope: &str) -> anyhow::Result<Vec<String>>;
    async fn list_groups(&self) -> anyhow::Result<Vec<String>>;
    /// Only `save_interval_ms` is hot-tunable (kv file_based); default no-op.
    async fn reconfigure(&self, _config: &Value) -> anyhow::Result<()> {
        Ok(())
    }
    async fn destroy(&self) -> anyhow::Result<()>;
}

pub struct KvStoreAdapter {
    storage: KvStore,
}

impl KvStoreAdapter {
    pub fn new(config: Option<Value>) -> Self {
        Self {
            storage: KvStore::new(config),
        }
    }
}

#[async_trait]
impl StateAdapter for KvStoreAdapter {
    async fn set(&self, scope: &str, key: &str, value: Value) -> anyhow::Result<StreamSetResult> {
        Ok(self
            .storage
            .set(scope.to_string(), key.to_string(), value)
            .await)
    }
    async fn get(&self, scope: &str, key: &str) -> anyhow::Result<Option<Value>> {
        Ok(self.storage.get(scope.to_string(), key.to_string()).await)
    }
    async fn delete(&self, scope: &str, key: &str) -> anyhow::Result<()> {
        self.storage
            .delete(scope.to_string(), key.to_string())
            .await;
        Ok(())
    }
    async fn update(
        &self,
        scope: &str,
        key: &str,
        ops: Vec<UpdateOp>,
    ) -> anyhow::Result<StreamUpdateResult> {
        Ok(self
            .storage
            .update(scope.to_string(), key.to_string(), ops)
            .await)
    }
    async fn compare_and_set(
        &self,
        scope: &str,
        key: &str,
        expected: Option<&Value>,
        value: Value,
    ) -> anyhow::Result<CompareAndSetOutcome> {
        Ok(self
            .storage
            .compare_and_set(scope.to_string(), key.to_string(), expected, value)
            .await)
    }

    async fn barrier_arrive(
        &self,
        scope: &str,
        key: &str,
        cfg: &crate::barrier::BarrierConfig,
        event: &Value,
    ) -> anyhow::Result<crate::barrier::Decision> {
        self.storage
            .barrier_arrive(scope.to_string(), key.to_string(), cfg, event)
            .await
            .map_err(|e| anyhow::anyhow!(e))
    }
    async fn list(&self, scope: &str) -> anyhow::Result<Vec<Value>> {
        Ok(self.storage.list(scope.to_string()).await)
    }
    async fn list_keys(&self, scope: &str) -> anyhow::Result<Vec<String>> {
        Ok(self.storage.list_keys(scope.to_string()).await)
    }
    async fn list_groups(&self) -> anyhow::Result<Vec<String>> {
        Ok(self.storage.list_groups().await)
    }
    async fn reconfigure(&self, config: &Value) -> anyhow::Result<()> {
        self.storage.reconfigure(config);
        Ok(())
    }
    async fn destroy(&self) -> anyhow::Result<()> {
        Ok(())
    }
}

// Validation bounds mirrored from `engine/src/update_ops.rs`. If you
// change one side, change both.
//   MAX_PATH_DEPTH    = 32
//   MAX_SEGMENT_BYTES = 256
//   MAX_VALUE_DEPTH   = 16
//   MAX_VALUE_KEYS    = 1024
// Prototype-pollution sinks: __proto__, constructor, prototype.
const JSON_UPDATE_SCRIPT: &str = r#"
    local json_decode = cjson.decode
    local json_encode = cjson.encode

    local MAX_PATH_DEPTH = 32
    local MAX_SEGMENT_BYTES = 256
    local MAX_VALUE_DEPTH = 16
    local MAX_VALUE_KEYS = 1024
    local PROTO = { __proto__ = true, constructor = true, prototype = true }
    local DOC_URL = 'https://iii.dev/docs/workers/iii-state#error-codes'

    local key = KEYS[1]
    local field = ARGV[1]
    local ops_json = ARGV[2]

    local old_value_str = redis.call('HGET', key, field)
    local old_value = {}
    if old_value_str then
        local ok, decoded = pcall(json_decode, old_value_str)
        if ok then
            old_value = decoded
        else
            return {'false', 'failed to decode existing JSON: ' .. tostring(decoded)}
        end
    end

    local ops = json_decode(ops_json)
    local current = json_decode(json_encode(old_value))
    local using_missing_default = old_value_str == nil
    local errors = {}

    -- get_path for legacy non-merge ops: collapses anything to a single
    -- first-level key for backward compat.
    local function get_path(path)
        if path == nil then return nil end
        if type(path) == 'string' then return path end
        if type(path) == 'table' then
            if path[1] then return path[1] end
            if path['0'] then return path['0'] end
        end
        return path
    end

    -- merge_path_segments: returns a Lua array of literal segments, or
    -- empty array meaning "root merge".
    local function merge_path_segments(path)
        if path == nil then return {} end
        if type(path) == 'string' then
            if path == '' then return {} end
            return { path }
        end
        if type(path) == 'table' then
            local out = {}
            for i, seg in ipairs(path) do
                out[i] = seg
            end
            return out
        end
        return {}
    end

    local function push_error(op_index, code, message)
        errors[#errors + 1] = {
            op_index = op_index,
            code = code,
            message = message,
            doc_url = DOC_URL,
        }
    end

    local function path_error_code(op_name, reason)
        return op_name .. '.path.' .. reason
    end

    local function json_type_name(value)
        if value == cjson.null then return 'null' end
        local value_type = type(value)
        if value_type == 'boolean' then return 'boolean' end
        if value_type == 'number' then return 'number' end
        if value_type == 'string' then return 'string' end
        if value_type == 'table' then
            if value[1] ~= nil then return 'array' end
            return 'object'
        end
        return value_type
    end

    local function path_label(path)
        if path == nil or path == '' then return 'root' end
        return path
    end

    -- Bracket-notation label for nested-segment paths. Mirrors the Rust
    -- helper `path_label_segments` in `engine/src/update_ops.rs` so the
    -- error messages produced by the two adapters match byte-for-byte.
    local function path_label_segments(segments)
        if segments == nil or #segments == 0 then return 'root' end
        return '[' .. table.concat(segments, ', ') .. ']'
    end

    local function field_path_segments(path)
        if path == nil or path == '' then return {} end
        return { path }
    end

    local function json_depth(value)
        if type(value) ~= 'table' then return 0 end
        local max = 0
        for _, v in pairs(value) do
            local d = json_depth(v)
            if d > max then max = d end
        end
        return 1 + max
    end

    local function validate_op_path(op_name, op_index, segments)
        if #segments > MAX_PATH_DEPTH then
            push_error(op_index, path_error_code(op_name, 'too_deep'),
                'Path depth ' .. #segments .. ' exceeds maximum of ' .. MAX_PATH_DEPTH)
            return false
        end
        for _, seg in ipairs(segments) do
            if type(seg) ~= 'string' or seg == '' then
                push_error(op_index, path_error_code(op_name, 'empty_segment'),
                    'Path contains an empty or non-string segment')
                return false
            end
            if #seg > MAX_SEGMENT_BYTES then
                push_error(op_index, path_error_code(op_name, 'segment_too_long'),
                    'Path segment of ' .. #seg .. ' bytes exceeds maximum of ' .. MAX_SEGMENT_BYTES)
                return false
            end
            if PROTO[seg] then
                push_error(op_index, path_error_code(op_name, 'proto_polluted'),
                    "Path segment '" .. seg .. "' is not allowed (prototype pollution).")
                return false
            end
        end
        return true
    end

    local function validate_merge_path(op_index, segments)
        return validate_op_path('merge', op_index, segments)
    end

    local function validate_merge_value(op_index, value)
        if type(value) ~= 'table' or value == cjson.null then
            push_error(op_index, 'merge.value.not_an_object',
                'Merge value must be a JSON object')
            return false
        end
        local key_count = 0
        for k, _ in pairs(value) do
            -- JSON arrays land as Lua arrays with numeric keys; reject.
            if type(k) ~= 'string' then
                push_error(op_index, 'merge.value.not_an_object',
                    'Merge value must be a JSON object')
                return false
            end
            if PROTO[k] then
                push_error(op_index, 'merge.value.proto_polluted',
                    'Merge value top-level key "' .. k .. '" is a prototype-pollution sink')
                return false
            end
            key_count = key_count + 1
            if key_count > MAX_VALUE_KEYS then
                push_error(op_index, 'merge.value.too_many_keys',
                    'Merge value has more than ' .. MAX_VALUE_KEYS .. ' top-level keys')
                return false
            end
        end
        if json_depth(value) > MAX_VALUE_DEPTH then
            push_error(op_index, 'merge.value.too_deep',
                'Merge value JSON nesting depth exceeds maximum of ' .. MAX_VALUE_DEPTH)
            return false
        end
        return true
    end

    -- "Object shape" check used at the IMP-003 root gate and inside
    -- `walk_or_create` to decide whether to walk into a node or replace
    -- it. Mirrors `json_type_name`'s convention (`value[1] ~= nil`
    -- means array, otherwise object) so empty Lua tables count as
    -- objects here. Aligns the Lua walk with the Rust path, which uses
    -- `matches!(v, Value::Object(_))` and replaces non-object
    -- intermediates (including arrays) with `Value::Object(Map::new())`.
    --
    -- Defined before `walk_or_create` (and other consumers) so the
    -- closure resolves it as an upvalue rather than a missing global.
    local function is_object_shape(value)
        if type(value) ~= 'table' or value == cjson.null then
            return false
        end
        return value[1] == nil
    end

    -- Walk segments inside `root`, replacing any non-object intermediate
    -- (null, scalar, OR array) with a fresh empty object. Mirrors the
    -- Rust `walk_or_create` (engine/src/update_ops.rs) which calls
    -- `*entry = Value::Object(Map::new())` whenever the intermediate is
    -- not already a `Value::Object(_)`. Without the array branch, Lua
    -- would walk into an existing array intermediate and produce a
    -- corrupted mixed-key form like `{1=1, 2=2, b=[42]}` for state
    -- `{"a": [1,2,3]}` + nested append `["a","b"]`.
    local function walk_or_create(root, segments)
        if not is_object_shape(root) then
            return nil  -- caller normalises root before invoking
        end
        local node = root
        for _, seg in ipairs(segments) do
            local next_node = node[seg]
            if not is_object_shape(next_node) then
                next_node = {}
                node[seg] = next_node
            end
            node = next_node
        end
        return node
    end

    local function initial_append_value(value)
        if type(value) == 'string' then
            return value
        end
        return {value}
    end

    -- Used by `append_to_target` to decide whether an existing leaf is
    -- appendable as an array. cjson loses the empty-`[]` vs empty-`{}`
    -- distinction across the encode/decode roundtrip (both materialise
    -- as a `{}` Lua table; Redis cjson does not expose `array_mt`), so
    -- this heuristic accepts empty tables as arrays. That matches the
    -- common case where users append into a freshly-stored `[]` leaf
    -- (e.g. `{"buffer": []}` + `append("buffer", x)`); the dual case
    -- where the leaf is stored as `{}` is documented as a known
    -- limitation of the Lua path. The IMP-003 root gate and
    -- `walk_or_create` use `is_object_shape` instead, so empty-document
    -- nested append still works.
    local function is_array(value)
        if type(value) ~= 'table' then
            return false
        end
        local max = 0
        local count = 0
        for k, _ in pairs(value) do
            if type(k) ~= 'number' or k < 1 or math.floor(k) ~= k then
                return false
            end
            if k > max then
                max = k
            end
            count = count + 1
        end
        return count == max
    end

    local function append_to_target(target, value, path, op_index)
        if target == nil or target == cjson.null then
            return true, initial_append_value(value)
        end
        if type(target) == 'string' then
            if type(value) == 'string' then
                return true, target .. value
            end
            push_error(op_index, 'append.type_mismatch',
                "Expected string append value at path '" .. path_label(path) .. "', got " .. json_type_name(value) .. ".")
            return false, target
        end
        if is_array(target) then
            table.insert(target, value)
            return true, target
        end
        push_error(op_index, 'append.type_mismatch',
            "Cannot append at path '" .. path_label(path) .. "': target is " .. json_type_name(target) .. ", expected array, string, null, or missing field.")
        return false, target
    end

    for op_index, op in ipairs(ops) do
        -- ipairs yields 1-based; mirror the engine's 0-based op_index.
        local zero_index = op_index - 1
        if op.type == 'set' then
            local path = get_path(op.path)
            if validate_op_path('set', zero_index, field_path_segments(path)) then
              if (path == '' or path == nil) and op.value ~= nil then
                current = op.value
                using_missing_default = false
              elseif type(current) == 'table' and current ~= cjson.null then
                if op.value == nil then
                    current[path] = cjson.null
                else
                    current[path] = op.value
                end
                using_missing_default = false
              else
                push_error(zero_index, 'set.target_not_object',
                    "Cannot set at path '" .. path_label(path) .. "': target is " .. json_type_name(current) .. ", expected object.")
              end
            end
        elseif op.type == 'merge' then
            local segments = merge_path_segments(op.path)
            if validate_merge_path(zero_index, segments) and
               validate_merge_value(zero_index, op.value) then
                if #segments == 0 then
                    -- Root merge — preserve existing semantics.
                    if type(current) == 'table' and current ~= cjson.null then
                        for k, v in pairs(op.value) do
                            current[k] = v
                        end
                        using_missing_default = false
                    end
                else
                    if type(current) ~= 'table' or current == cjson.null then
                        current = {}
                    end
                    local target = walk_or_create(current, segments)
                    if target ~= nil then
                        for k, v in pairs(op.value) do
                            target[k] = v
                        end
                        using_missing_default = false
                    end
                end
            end
        elseif op.type == 'increment' then
            local path = get_path(op.path)
            if validate_op_path('increment', zero_index, field_path_segments(path)) then
              if path == '' or path == nil then
                if using_missing_default then
                    current = op.by
                    using_missing_default = false
                elseif type(current) == 'number' then
                    current = current + op.by
                    using_missing_default = false
                else
                    push_error(zero_index, 'increment.not_number',
                        "Expected number at path '" .. path_label(path) .. "', got " .. json_type_name(current) .. ".")
                end
              elseif type(current) == 'table' and current ~= cjson.null then
                local val = current[path]
                if val == nil then
                    current[path] = op.by
                    using_missing_default = false
                elseif type(val) == 'number' then
                    current[path] = val + op.by
                    using_missing_default = false
                else
                    push_error(zero_index, 'increment.not_number',
                        "Expected number at path '" .. path_label(path) .. "', got " .. json_type_name(val) .. ".")
                end
              else
                push_error(zero_index, 'increment.target_not_object',
                    "Cannot increment at path '" .. path_label(path) .. "': target is " .. json_type_name(current) .. ", expected object.")
              end
            end
        elseif op.type == 'decrement' then
            local path = get_path(op.path)
            if validate_op_path('decrement', zero_index, field_path_segments(path)) then
              if path == '' or path == nil then
                if using_missing_default then
                    current = -op.by
                    using_missing_default = false
                elseif type(current) == 'number' then
                    current = current - op.by
                    using_missing_default = false
                else
                    push_error(zero_index, 'decrement.not_number',
                        "Expected number at path '" .. path_label(path) .. "', got " .. json_type_name(current) .. ".")
                end
              elseif type(current) == 'table' and current ~= cjson.null then
                local val = current[path]
                if val == nil then
                    current[path] = -op.by
                    using_missing_default = false
                elseif type(val) == 'number' then
                    current[path] = val - op.by
                    using_missing_default = false
                else
                    push_error(zero_index, 'decrement.not_number',
                        "Expected number at path '" .. path_label(path) .. "', got " .. json_type_name(val) .. ".")
                end
              else
                push_error(zero_index, 'decrement.target_not_object',
                    "Cannot decrement at path '" .. path_label(path) .. "': target is " .. json_type_name(current) .. ", expected object.")
              end
            end
        elseif op.type == 'append' then
            -- Validation order is load-bearing (mirror of update_ops.rs):
            --   1. validate_op_path  (bounds + proto-pollution)
            --   2. root-is-object    (before walk_or_create can mutate)
            --   3. walk_or_create    (nested only)
            --   4. leaf-type matrix  (FR-11)
            local segments = merge_path_segments(op.path)
            if validate_op_path('append', zero_index, segments) then
              if #segments == 0 then
                -- Root append: legacy semantics preserved.
                local target_root = using_missing_default and cjson.null or current
                local changed, next_value = append_to_target(target_root, op.value, 'root', zero_index)
                if changed then
                    current = next_value
                    using_missing_default = false
                end
              elseif not is_object_shape(current) then
                -- Non-empty path requires object root (IMP-003). Empty
                -- Lua tables count as objects per `is_object_shape`, so
                -- empty-document nested append succeeds (matching the
                -- Rust path's `Value::Object({})` initialization).
                push_error(zero_index, 'append.target_not_object',
                    "Cannot append at path '" .. path_label_segments(segments) .. "': target is " .. json_type_name(current) .. ", expected object.")
              elseif #segments == 1 then
                -- Single-segment path: back-compat with the legacy
                -- single-string `FieldPath` semantics — `initial_append_value`
                -- keeps the string-concat tier for missing leaves.
                local leaf_key = segments[1]
                local existing_val = current[leaf_key]
                if existing_val ~= nil and existing_val ~= cjson.null then
                    local changed, next_value = append_to_target(existing_val, op.value, leaf_key, zero_index)
                    if changed then
                        current[leaf_key] = next_value
                    end
                else
                    current[leaf_key] = initial_append_value(op.value)
                end
                using_missing_default = false
              else
                -- Nested path: walk parent (creating intermediates), then
                -- operate on the leaf key. FR-11 nested-path rule: missing
                -- leaf is ALWAYS an array (no string-concat tier).
                local parent_segments = {}
                for i = 1, #segments - 1 do
                    parent_segments[i] = segments[i]
                end
                local leaf_key = segments[#segments]
                local parent_map = walk_or_create(current, parent_segments)
                if parent_map ~= nil then
                    local existing_val = parent_map[leaf_key]
                    if existing_val ~= nil and existing_val ~= cjson.null then
                        local changed, next_value = append_to_target(existing_val, op.value, leaf_key, zero_index)
                        if changed then
                            parent_map[leaf_key] = next_value
                        end
                    else
                        -- FR-11: nested-path missing leaf is always an array.
                        parent_map[leaf_key] = { op.value }
                    end
                    using_missing_default = false
                end
              end
            end
        elseif op.type == 'remove' then
            local path = get_path(op.path)
            if validate_op_path('remove', zero_index, field_path_segments(path)) then
              if path == '' or path == nil then
                current = cjson.null
                using_missing_default = false
              elseif type(current) == 'table' and current ~= cjson.null then
                current[path] = nil
                using_missing_default = false
              else
                push_error(zero_index, 'remove.target_not_object',
                    "Cannot remove at path '" .. path_label(path) .. "': target is " .. json_type_name(current) .. ", expected object.")
              end
            end
        end
    end

    local new_value_str = json_encode(current)
    redis.call('HSET', key, field, new_value_str)

    -- Return tuple shape:
    --   {'true', old_value_str, new_value_str, errors_json}
    -- errors_json is omitted (4 elements only when present) when no
    -- errors occurred, preserving backward compatibility with adapters
    -- that expect 3 elements.
    if #errors == 0 then
        return {'true', old_value_str or '', new_value_str}
    else
        return {'true', old_value_str or '', new_value_str, json_encode(errors)}
    end
"#;

pub struct RedisAdapter {
    publisher: Arc<Mutex<ConnectionManager>>,
}

impl RedisAdapter {
    pub async fn new(redis_url: String) -> anyhow::Result<Self> {
        let client = Client::open(redis_url.as_str())?;
        let manager = timeout(REDIS_CONNECTION_TIMEOUT, client.get_connection_manager())
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "Redis connection timed out after {:?}. Please ensure Redis is running at: {}",
                    REDIS_CONNECTION_TIMEOUT,
                    redis_url
                )
            })?
            .map_err(|e| anyhow::anyhow!("Failed to connect to Redis at {}: {}", redis_url, e))?;

        let publisher = Arc::new(Mutex::new(manager));
        Ok(Self { publisher })
    }
}

#[async_trait]
impl StateAdapter for RedisAdapter {
    /// Compare parsed JSON values, then commit only if the watched scope has
    /// not changed. This gives Redis the same semantic equality as the KV
    /// adapter instead of depending on object-key serialization order.
    async fn compare_and_set(
        &self,
        scope: &str,
        key: &str,
        expected: Option<&Value>,
        value: Value,
    ) -> anyhow::Result<CompareAndSetOutcome> {
        let scope_key = format!("state:{}", scope);
        let next = serde_json::to_string(&value)
            .map_err(|e| anyhow::anyhow!("Failed to serialize value: {}", e))?;

        // ponytail: WATCH is scope-wide and retries are capped at 8; use
        // per-field version keys if unrelated writes cause contention.
        for _ in 0..REDIS_CAS_MAX_ATTEMPTS {
            let mut conn = self.publisher.lock().await;
            redis::cmd("WATCH")
                .arg(&scope_key)
                .query_async::<()>(&mut *conn)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to watch Redis CAS value: {e}"))?;

            let encoded: Option<String> = match conn.hget(&scope_key, key).await {
                Ok(encoded) => encoded,
                Err(error) => {
                    let _ = redis::cmd("UNWATCH").query_async::<()>(&mut *conn).await;
                    return Err(anyhow::anyhow!("Failed to read Redis CAS value: {error}"));
                }
            };
            let current = match encoded.as_deref().map(serde_json::from_str).transpose() {
                Ok(current) => current,
                Err(error) => {
                    let _ = redis::cmd("UNWATCH").query_async::<()>(&mut *conn).await;
                    return Err(anyhow::anyhow!("Failed to parse Redis CAS value: {error}"));
                }
            };

            if !cas_matches(expected, current.as_ref()) {
                redis::cmd("UNWATCH")
                    .query_async::<()>(&mut *conn)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to unwatch Redis CAS value: {e}"))?;
                return Ok(CompareAndSetOutcome::NotSwapped {
                    current: current.unwrap_or(Value::Null),
                });
            }

            let committed: Option<(usize,)> = redis::pipe()
                .atomic()
                .cmd("HSET")
                .arg(&scope_key)
                .arg(key)
                .arg(&next)
                .query_async(&mut *conn)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to compare-and-set value in Redis: {e}"))?;
            if committed.is_some() {
                return Ok(CompareAndSetOutcome::Swapped { old_value: current });
            }
        }
        anyhow::bail!(
            "Failed to compare-and-set value in Redis after {REDIS_CAS_MAX_ATTEMPTS} attempts"
        )
    }

    /// Redis has the atomicity for this (a Lua script over the hash field),
    /// but the decision logic lives in Rust and porting it to Lua would mean
    /// maintaining the same rules twice — the kind of divergence that shows up
    /// as "the barrier behaves differently in prod". Refuse clearly instead;
    /// the port is a bounded task if a redis-backed deployment needs it.
    async fn barrier_arrive(
        &self,
        _scope: &str,
        _key: &str,
        cfg: &crate::barrier::BarrierConfig,
        _event: &Value,
    ) -> anyhow::Result<crate::barrier::Decision> {
        anyhow::bail!(
            "barrier `{}` needs the kv adapter: the redis adapter has no atomic \
             read-modify-write for it yet, and a non-atomic one would let two arrivals \
             both complete the same barrier",
            cfg.id
        )
    }

    async fn set(&self, scope: &str, key: &str, value: Value) -> anyhow::Result<StreamSetResult> {
        let scope_key: String = format!("state:{}", scope);
        let mut conn = self.publisher.lock().await;
        let serialized = serde_json::to_string(&value)
            .map_err(|e| anyhow::anyhow!("Failed to serialize value: {}", e))?;

        // Use Lua script for atomic get-and-set operation
        // This script atomically gets the old value and sets the new value
        let script = redis::Script::new(
            r#"
                local old_value = redis.call('HGET', KEYS[1], ARGV[1])
                redis.call('HSET', KEYS[1], ARGV[1], ARGV[2])
                return old_value
            "#,
        );

        let old_value_str: Option<String> = script
            .key(&scope_key)
            .arg(key)
            .arg(&serialized)
            .invoke_async(&mut *conn)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to atomically set value in Redis: {}", e))?;

        let old_value = old_value_str.map(|s| serde_json::from_str(&s).unwrap_or(Value::Null));

        Ok(StreamSetResult {
            old_value,
            new_value: value,
        })
    }

    async fn get(&self, scope: &str, key: &str) -> anyhow::Result<Option<Value>> {
        let scope_key = format!("state:{}", scope);
        let mut conn = self.publisher.lock().await;

        match conn.hget::<_, _, Option<String>>(&scope_key, &key).await {
            Ok(Some(s)) => serde_json::from_str(&s)
                .map_err(|e| anyhow::anyhow!("Failed to deserialize value from Redis: {}", e))
                .map(Some),
            Ok(None) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("Failed to get value from Redis: {}", e)),
        }
    }

    async fn update(
        &self,
        scope: &str,
        key: &str,
        ops: Vec<UpdateOp>,
    ) -> anyhow::Result<StreamUpdateResult> {
        let mut conn = self.publisher.lock().await;
        let scope_key = format!("state:{}", scope);

        // Serialize operations to JSON
        let ops_json = serde_json::to_string(&ops)
            .map_err(|e| anyhow::anyhow!("Failed to serialize update operations: {}", e))?;

        // Use a single Lua script that atomically gets, applies operations, and sets.
        let script = redis::Script::new(JSON_UPDATE_SCRIPT);

        let result: redis::RedisResult<Vec<String>> = script
            .key(&scope_key)
            .arg(key)
            .arg(&ops_json)
            .invoke_async(&mut *conn)
            .await;

        match result {
            Ok(values) if values.len() >= 2 => {
                // Check if the Lua update script reported a failure.
                if values[0] == "false" {
                    return Err(anyhow::anyhow!(
                        "Redis atomic update script failed: {}",
                        values.get(1).map_or("unknown error", String::as_str)
                    ));
                }

                if values.len() == 3 || values.len() == 4 {
                    let old_value = if values[1].is_empty() {
                        None
                    } else {
                        serde_json::from_str(&values[1]).map_err(|e| {
                            anyhow::anyhow!("Failed to deserialize old value: {}", e)
                        })?
                    };

                    let new_value = serde_json::from_str(&values[2])
                        .map_err(|e| anyhow::anyhow!("Failed to deserialize new value: {}", e))?;

                    let errors = if values.len() == 4 && !values[3].is_empty() {
                        serde_json::from_str(&values[3]).map_err(|e| {
                            anyhow::anyhow!("Failed to deserialize update errors: {}", e)
                        })?
                    } else {
                        Vec::new()
                    };

                    Ok(StreamUpdateResult {
                        old_value,
                        new_value,
                        errors,
                    })
                } else {
                    Err(anyhow::anyhow!(
                        "Unexpected return value from update script: expected 3 or 4 values, got {}",
                        values.len()
                    ))
                }
            }
            Err(e) => Err(anyhow::anyhow!("Redis atomic update script failed: {}", e)),
            _ => Err(anyhow::anyhow!(
                "Unexpected return value from update script"
            )),
        }
    }

    async fn delete(&self, scope: &str, key: &str) -> anyhow::Result<()> {
        let scope_key = format!("state:{}", scope);
        let mut conn = self.publisher.lock().await;

        conn.hdel::<_, String, ()>(&scope_key, key.to_string())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to delete value from Redis: {}", e))?;
        Ok(())
    }

    async fn list(&self, scope: &str) -> anyhow::Result<Vec<Value>> {
        let scope_key = format!("state:{}", scope);
        let mut conn = self.publisher.lock().await;

        let values = conn
            .hgetall::<String, HashMap<String, String>>(scope_key)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to get group from Redis: {}", e))?;

        let mut result = Vec::new();
        for v in values.into_values() {
            result.push(
                serde_json::from_str(&v)
                    .map_err(|e| anyhow::anyhow!("Failed to deserialize value: {}", e))?,
            );
        }
        Ok(result)
    }

    async fn list_keys(&self, scope: &str) -> anyhow::Result<Vec<String>> {
        let scope_key = format!("state:{}", scope);
        let mut conn = self.publisher.lock().await;

        conn.hkeys::<_, Vec<String>>(&scope_key)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list keys from Redis: {}", e))
    }

    async fn list_groups(&self) -> anyhow::Result<Vec<String>> {
        let mut conn = self.publisher.lock().await;
        let mut cursor = 0u64;
        let mut groups = Vec::new();

        loop {
            let (new_cursor, keys): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg("state:*")
                .arg("COUNT")
                .arg(100)
                .query_async(&mut *conn)
                .await?;

            for key in keys {
                if let Some(scope) = key.strip_prefix("state:") {
                    groups.push(scope.to_string());
                }
            }

            cursor = new_cursor;
            if cursor == 0 {
                break;
            }
        }

        Ok(groups)
    }

    async fn destroy(&self) -> anyhow::Result<()> {
        tracing::debug!("Destroying RedisAdapter");
        Ok(())
    }
}

/// Build the adapter named in the config. Unknown names error (parity with the
/// builtin's adapter registry). `bridge` is intentionally not ported.
pub async fn build_adapter(config: &StateConfig) -> anyhow::Result<Arc<dyn StateAdapter>> {
    match config.effective_adapter_name() {
        "kv" => Ok(Arc::new(KvStoreAdapter::new(config.adapter_config()))),
        "redis" => {
            let url = config
                .adapter
                .as_ref()
                .and_then(|a| a.config.as_ref())
                .and_then(|c| c.get("redis_url"))
                .and_then(|v| v.as_str())
                .unwrap_or(DEFAULT_REDIS_URL)
                .to_string();
            Ok(Arc::new(RedisAdapter::new(url).await?))
        }
        other => anyhow::bail!("unknown state adapter '{other}' (expected 'kv' or 'redis')"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::StateConfig;

    #[test]
    fn cas_uses_json_value_equality() {
        let stored = serde_json::from_str(r#"{"a":1,"b":2}"#).unwrap();
        let reordered = serde_json::from_str(r#"{"b":2,"a":1}"#).unwrap();
        assert!(cas_matches(Some(&reordered), Some(&stored)));
        assert!(cas_matches(None, Some(&Value::Null)));
        assert!(cas_matches(Some(&Value::Null), None));
        assert!(!cas_matches(
            Some(&serde_json::json!([])),
            Some(&serde_json::json!({}))
        ));
    }

    #[test]
    fn redis_exec_result_distinguishes_watch_abort() {
        let committed: Option<(usize,)> =
            redis::from_redis_value(&redis::Value::Array(vec![redis::Value::Int(1)])).unwrap();
        let aborted: Option<(usize,)> = redis::from_redis_value(&redis::Value::Nil).unwrap();
        assert_eq!(committed, Some((1,)));
        assert_eq!(aborted, None);
    }

    #[tokio::test]
    async fn kv_adapter_set_get_delete_update_list_roundtrip() {
        let a = KvStoreAdapter::new(None);
        a.set("s", "k", serde_json::json!({"count": 0}))
            .await
            .unwrap();
        assert_eq!(
            a.get("s", "k").await.unwrap(),
            Some(serde_json::json!({"count": 0}))
        );
        let updated = a
            .update(
                "s",
                "k",
                vec![iii_helpers::stream::UpdateOp::increment("count", 2)],
            )
            .await
            .unwrap();
        assert_eq!(updated.new_value["count"], 2);
        assert_eq!(a.list("s").await.unwrap().len(), 1);
        assert_eq!(a.list_groups().await.unwrap(), vec!["s".to_string()]);
        a.delete("s", "k").await.unwrap();
        assert_eq!(a.get("s", "k").await.unwrap(), None);
    }

    #[tokio::test]
    async fn build_adapter_defaults_to_kv_and_rejects_unknown() {
        let kv = build_adapter(&StateConfig::default()).await;
        assert!(kv.is_ok());
        let bad: StateConfig =
            serde_json::from_value(serde_json::json!({"adapter": {"name": "postgres"}})).unwrap();
        assert!(build_adapter(&bad).await.is_err());
    }
}
