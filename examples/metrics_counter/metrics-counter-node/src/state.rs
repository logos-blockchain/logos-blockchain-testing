use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use prometheus::{IntCounter, IntGauge};
use serde::Serialize;
use tokio::sync::RwLock;

#[derive(Clone, Debug, Serialize)]
pub struct CounterView {
    pub value: u64,
}

#[derive(Clone)]
pub struct CounterState {
    ready: Arc<RwLock<bool>>,
    value: Arc<AtomicU64>,
    increments_total: IntCounter,
    counter_value: IntGauge,
}

impl CounterState {
    pub fn new() -> Self {
        let increments_total = IntCounter::new(
            "metrics_counter_increments_total",
            "Total increment operations",
        )
        .expect("metrics_counter_increments_total");
        let counter_value =
            IntGauge::new("metrics_counter_value", "Current counter value").expect("counter gauge");

        let _ = prometheus::register(Box::new(increments_total.clone()));
        let _ = prometheus::register(Box::new(counter_value.clone()));

        Self {
            ready: Arc::new(RwLock::new(false)),
            value: Arc::new(AtomicU64::new(0)),
            increments_total,
            counter_value,
        }
    }

    pub async fn set_ready(&self, value: bool) {
        *self.ready.write().await = value;
    }

    pub async fn is_ready(&self) -> bool {
        *self.ready.read().await
    }

    pub fn increment(&self) -> CounterView {
        let value = self.value.fetch_add(1, Ordering::SeqCst) + 1;
        self.increments_total.inc();
        self.counter_value.set(value as i64);
        CounterView { value }
    }

    pub fn view(&self) -> CounterView {
        CounterView {
            value: self.value.load(Ordering::SeqCst),
        }
    }
}
