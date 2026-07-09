//! RabbitMQ exchange/queue naming scheme.
//!
//! 1:1 port of `engine/src/workers/queue/adapters/rabbitmq/naming.rs` — pure
//! string formatting, no engine dependency, ports verbatim.

#![cfg(feature = "rabbitmq")]

pub const EXCHANGE_PREFIX: &str = "iii";

pub struct RabbitNames {
    pub topic: String,
}

impl RabbitNames {
    pub fn new(topic: impl Into<String>) -> Self {
        Self {
            topic: topic.into(),
        }
    }

    pub fn exchange(&self) -> String {
        format!("{}.{}.exchange", EXCHANGE_PREFIX, self.topic)
    }

    pub fn queue(&self) -> String {
        format!("{}.{}.queue", EXCHANGE_PREFIX, self.topic)
    }

    pub fn function_queue(&self, function_id: &str) -> String {
        format!("{}.{}.{}.queue", EXCHANGE_PREFIX, self.topic, function_id)
    }

    pub fn function_dlq(&self, function_id: &str) -> String {
        format!("{}.{}.{}.dlq", EXCHANGE_PREFIX, self.topic, function_id)
    }

    pub fn dlq(&self) -> String {
        format!("{}.{}.dlq", EXCHANGE_PREFIX, self.topic)
    }
}

pub struct FnQueueNames {
    pub name: String,
}

impl FnQueueNames {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }

    pub fn exchange(&self) -> String {
        format!("{}.{}.exchange", EXCHANGE_PREFIX, self.name)
    }

    pub fn queue(&self) -> String {
        format!("{}.{}.queue", EXCHANGE_PREFIX, self.name)
    }

    pub fn retry_exchange(&self) -> String {
        format!("{}.{}.retry", EXCHANGE_PREFIX, self.name)
    }

    pub fn retry_queue(&self) -> String {
        format!("{}.{}.retry.queue", EXCHANGE_PREFIX, self.name)
    }

    pub fn dlq_exchange(&self) -> String {
        format!("{}.{}.dlq", EXCHANGE_PREFIX, self.name)
    }

    pub fn dlq(&self) -> String {
        format!("{}.{}.dlq.queue", EXCHANGE_PREFIX, self.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rabbit_names() {
        let names = RabbitNames::new("user.created");

        assert_eq!(names.exchange(), "iii.user.created.exchange");
        assert_eq!(names.queue(), "iii.user.created.queue");
        assert_eq!(names.dlq(), "iii.user.created.dlq");
    }

    #[test]
    fn test_fn_queue_names_exchange() {
        let names = FnQueueNames::new("orders");
        assert_eq!(names.exchange(), "iii.orders.exchange");
    }

    #[test]
    fn test_fn_queue_names_queue() {
        let names = FnQueueNames::new("orders");
        assert_eq!(names.queue(), "iii.orders.queue");
    }

    #[test]
    fn test_fn_queue_names_retry_exchange() {
        let names = FnQueueNames::new("orders");
        assert_eq!(names.retry_exchange(), "iii.orders.retry");
    }

    #[test]
    fn test_fn_queue_names_retry_queue() {
        let names = FnQueueNames::new("orders");
        assert_eq!(names.retry_queue(), "iii.orders.retry.queue");
    }

    #[test]
    fn test_fn_queue_names_dlq_exchange() {
        let names = FnQueueNames::new("orders");
        assert_eq!(names.dlq_exchange(), "iii.orders.dlq");
    }

    #[test]
    fn test_fn_queue_names_dlq() {
        let names = FnQueueNames::new("orders");
        assert_eq!(names.dlq(), "iii.orders.dlq.queue");
    }

    #[test]
    fn test_fn_queue_names_with_dots() {
        let names = FnQueueNames::new("payment.processing");
        assert_eq!(names.exchange(), "iii.payment.processing.exchange");
        assert_eq!(names.queue(), "iii.payment.processing.queue");
        assert_eq!(names.retry_exchange(), "iii.payment.processing.retry");
        assert_eq!(names.retry_queue(), "iii.payment.processing.retry.queue");
        assert_eq!(names.dlq_exchange(), "iii.payment.processing.dlq");
        assert_eq!(names.dlq(), "iii.payment.processing.dlq.queue");
    }
}
