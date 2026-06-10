//! The ONLY file (with channels.rs) that touches iii_sdk for bus traffic:
//! `Bus::trigger` → `iii.trigger`, `Bus::register_function` →
//! `iii.register_function(RegisterFunction::new_async)`, `Bus::register_trigger`
//! → `iii.register_trigger(RegisterTriggerInput)`, and
//! `Bus::register_trigger_type` → `iii.register_trigger_type(RegisterTriggerType)`
//! with an async `TriggerHandler` adapted onto our sync callbacks.
//! Signatures verified against iii-sdk 0.16.0-next.2.
use std::sync::Arc;

use iii_sdk::{
    IIIError, RegisterFunction, RegisterTriggerInput, RegisterTriggerType, TriggerConfig,
    TriggerHandler, TriggerRequest, III,
};
use serde_json::Value;

use crate::bus::{Bus, BusError, Handler, TriggerBinding, TriggerTypeCallbacks};

pub struct SdkBus {
    pub iii: III,
}

fn to_bus_error(err: IIIError) -> BusError {
    match err {
        IIIError::Timeout => BusError::Timeout,
        // engine/src/engine/mod.rs sends "function_not_found" (lowercase) for a
        // missing function; NOT a bare "NOT_FOUND", which the configuration
        // worker uses for missing entries and must stay Coded.
        IIIError::Remote { code, message, .. }
            if code.eq_ignore_ascii_case("function_not_found") =>
        {
            BusError::FunctionNotFound(message)
        }
        IIIError::Remote { code, message, .. } => BusError::Coded { code, message },
        other => BusError::Transport(other.to_string()),
    }
}

fn to_iii_error(err: BusError) -> IIIError {
    match err {
        BusError::Coded { code, message } => IIIError::Remote {
            code,
            message,
            stacktrace: None,
        },
        BusError::Timeout => IIIError::Timeout,
        other => IIIError::Handler(other.to_string()),
    }
}

type BindingCallback = Arc<dyn Fn(&TriggerBinding) + Send + Sync>;

/// Adapts the SDK's async `TriggerHandler` onto the bus seam's sync callbacks.
struct CallbackTriggerHandler {
    on_register: BindingCallback,
    on_unregister: BindingCallback,
}

fn to_binding(config: TriggerConfig) -> TriggerBinding {
    TriggerBinding {
        id: config.id,
        function_id: config.function_id,
        config: config.config,
    }
}

#[async_trait::async_trait]
impl TriggerHandler for CallbackTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), IIIError> {
        (self.on_register)(&to_binding(config));
        Ok(())
    }
    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), IIIError> {
        (self.on_unregister)(&to_binding(config));
        Ok(())
    }
}

#[async_trait::async_trait]
impl Bus for SdkBus {
    async fn trigger(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: Option<u64>,
    ) -> Result<Value, BusError> {
        self.iii
            .trigger(TriggerRequest {
                function_id: function_id.into(),
                payload,
                action: None,
                timeout_ms,
            })
            .await
            .map_err(to_bus_error)
    }

    fn register_function(&self, id: &str, handler: Handler) {
        self.iii.register_function(
            id.to_string(),
            RegisterFunction::new_async(move |input: Value| {
                let handler = handler.clone();
                async move { handler(input).await.map_err(to_iii_error) }
            }),
        );
    }

    fn register_trigger(&self, trigger_type: &str, function_id: &str, config: Value) {
        let _ = self.iii.register_trigger(RegisterTriggerInput {
            trigger_type: trigger_type.into(),
            function_id: function_id.into(),
            config,
            metadata: None,
        });
    }

    fn register_trigger_type(&self, id: &str, description: &str, callbacks: TriggerTypeCallbacks) {
        let handler = CallbackTriggerHandler {
            on_register: callbacks.on_register,
            on_unregister: callbacks.on_unregister,
        };
        let _ = self
            .iii
            .register_trigger_type(RegisterTriggerType::new(id, description, handler));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_function_maps_only_the_engines_function_not_found_code() {
        let missing_fn = to_bus_error(IIIError::Remote {
            code: "function_not_found".into(),
            message: "provider::x::stream".into(),
            stacktrace: None,
        });
        assert!(matches!(missing_fn, BusError::FunctionNotFound(_)));

        // the configuration worker's missing-entry code must stay Coded
        let missing_entry = to_bus_error(IIIError::Remote {
            code: "NOT_FOUND".into(),
            message: "configuration llm-router".into(),
            stacktrace: None,
        });
        assert!(matches!(missing_entry, BusError::Coded { code, .. } if code == "NOT_FOUND"));
    }
}
