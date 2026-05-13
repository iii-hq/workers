//! Bus trigger abstraction so adapters can be unit-tested without a live engine.
//!
//! `IIITrigger` mirrors the slice of `iii_sdk::III` that store adapters need.
//! Real code uses the blanket impl on `iii_sdk::III`; tests pass a mock that
//! records calls and returns canned responses.

use iii_sdk::{IIIError, TriggerRequest};
use serde_json::Value;

#[async_trait::async_trait]
pub trait IIITrigger: Send + Sync + 'static {
    async fn trigger(&self, request: TriggerRequest) -> Result<Value, IIIError>;
}

#[async_trait::async_trait]
impl IIITrigger for iii_sdk::III {
    async fn trigger(&self, request: TriggerRequest) -> Result<Value, IIIError> {
        iii_sdk::III::trigger(self, request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Compile-time assertion: `iii_sdk::III` implements `IIITrigger`.
    fn _assert_iii_implements_trait() {
        fn takes<T: IIITrigger>(_: &T) {}
        let _: fn(&iii_sdk::III) = takes;
    }
}
