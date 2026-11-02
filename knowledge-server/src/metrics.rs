use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

pub const LATENCY_MS: &[u64] = &[
    5, 10, 25, 50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 30_000, 60_000, 120_000,
];
pub const SIZES: &[u64] = &[
    10, 50, 100, 500, 1_000, 5_000, 10_000, 25_000, 50_000, 100_000, 200_000,
];

pub struct Histogram {
    bounds: &'static [u64],
    buckets: Box<[AtomicU64]>,
    count: AtomicU64,
    sum: AtomicU64,
}

impl Histogram {
    pub fn new(bounds: &'static [u64]) -> Self {
        Self {
            bounds,
            buckets: bounds.iter().map(|_| AtomicU64::new(0)).collect(),
            count: AtomicU64::new(0),
            sum: AtomicU64::new(0),
        }
    }

    pub fn observe(&self, elapsed: Duration) {
        self.record(elapsed.as_millis() as u64);
    }

    pub fn record(&self, value: u64) {
        if let Some(index) = self.bounds.iter().position(|bound| value <= *bound) {
            self.buckets[index].fetch_add(1, Ordering::Relaxed);
        }
        self.count.fetch_add(1, Ordering::Relaxed);
        self.sum.fetch_add(value, Ordering::Relaxed);
    }

    fn render(&self, out: &mut String, name: &str, labels: &str) {
        let mut cumulative = 0;
        for (bound, bucket) in self.bounds.iter().zip(self.buckets.iter()) {
            cumulative += bucket.load(Ordering::Relaxed);
            let _ = writeln!(
                out,
                "{name}_bucket{{{}le=\"{bound}\"}} {cumulative}",
                with_comma(labels)
            );
        }
        let count = self.count.load(Ordering::Relaxed);
        let _ = writeln!(
            out,
            "{name}_bucket{{{}le=\"+Inf\"}} {count}",
            with_comma(labels)
        );
        let _ = writeln!(
            out,
            "{name}_sum{} {}",
            braced(labels),
            self.sum.load(Ordering::Relaxed)
        );
        let _ = writeln!(out, "{name}_count{} {count}", braced(labels));
    }
}

fn with_comma(labels: &str) -> String {
    if labels.is_empty() {
        String::new()
    } else {
        format!("{labels},")
    }
}

fn braced(labels: &str) -> String {
    if labels.is_empty() {
        String::new()
    } else {
        format!("{{{labels}}}")
    }
}

fn label_set(labels: &[(&str, &str)]) -> String {
    labels
        .iter()
        .map(|(key, value)| {
            format!(
                "{key}=\"{}\"",
                value.replace('\\', "\\\\").replace('"', "\\\"")
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}

pub struct HistogramVec {
    name: &'static str,
    bounds: &'static [u64],
    series: Mutex<BTreeMap<String, Arc<Histogram>>>,
}

impl HistogramVec {
    pub const fn new(name: &'static str, bounds: &'static [u64]) -> Self {
        Self {
            name,
            bounds,
            series: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn with(&self, labels: &[(&str, &str)]) -> Arc<Histogram> {
        let mut series = self.series.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(
            series
                .entry(label_set(labels))
                .or_insert_with(|| Arc::new(Histogram::new(self.bounds))),
        )
    }

    fn render(&self, out: &mut String) {
        let series = self.series.lock().unwrap_or_else(|e| e.into_inner());
        if series.is_empty() {
            return;
        }
        let _ = writeln!(out, "# TYPE {} histogram", self.name);
        for (labels, histogram) in series.iter() {
            histogram.render(out, self.name, labels);
        }
    }
}

pub struct CounterVec {
    name: &'static str,
    series: Mutex<BTreeMap<String, u64>>,
}

impl CounterVec {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            series: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn inc(&self, labels: &[(&str, &str)]) {
        self.add(labels, 1);
    }

    pub fn add(&self, labels: &[(&str, &str)], amount: u64) {
        *self
            .series
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(label_set(labels))
            .or_insert(0) += amount;
    }

    pub fn get(&self, labels: &[(&str, &str)]) -> u64 {
        self.series
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&label_set(labels))
            .copied()
            .unwrap_or(0)
    }

    fn render(&self, out: &mut String) {
        render_series(
            out,
            self.name,
            "counter",
            &self.series.lock().unwrap_or_else(|e| e.into_inner()),
        );
    }
}

pub struct GaugeVec {
    name: &'static str,
    series: Mutex<BTreeMap<String, u64>>,
}

impl GaugeVec {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            series: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn set(&self, labels: &[(&str, &str)], value: u64) {
        self.series
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(label_set(labels), value);
    }

    pub fn remove(&self, labels: &[(&str, &str)]) {
        self.series
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&label_set(labels));
    }

    fn render(&self, out: &mut String) {
        render_series(
            out,
            self.name,
            "gauge",
            &self.series.lock().unwrap_or_else(|e| e.into_inner()),
        );
    }
}

fn render_series(out: &mut String, name: &str, kind: &str, series: &BTreeMap<String, u64>) {
    if series.is_empty() {
        return;
    }
    let _ = writeln!(out, "# TYPE {name} {kind}");
    for (labels, value) in series {
        let _ = writeln!(out, "{name}{} {value}", braced(labels));
    }
}

pub static TURN_STAGE: HistogramVec =
    HistogramVec::new("harvest_turn_stage_milliseconds", LATENCY_MS);
pub static TURNS: CounterVec = CounterVec::new("harvest_turns_total");
pub static TURN_REJECTIONS: CounterVec = CounterVec::new("harvest_turns_rejected_total");
pub static CONTEXT: HistogramVec = HistogramVec::new("harvest_turn_context_size", SIZES);
pub static LLM_CALLS: CounterVec = CounterVec::new("harvest_llm_calls_total");
pub static LLM_BREAKER_OPEN: GaugeVec = GaugeVec::new("harvest_llm_breaker_open");
pub static CLUSTER_LAPSES: CounterVec = CounterVec::new("harvest_cluster_lapses_total");
pub static MESH_DROPPED: CounterVec = CounterVec::new("harvest_mesh_batches_dropped_total");
pub static MESH_RESYNCS: CounterVec = CounterVec::new("harvest_mesh_resyncs_total");
pub static MESH_LINKS: GaugeVec = GaugeVec::new("harvest_mesh_links");

pub static TURN_DURATION: LazyLock<Histogram> = LazyLock::new(|| Histogram::new(LATENCY_MS));

pub fn render(out: &mut String) {
    TURN_STAGE.render(out);
    CONTEXT.render(out);
    TURNS.render(out);
    TURN_REJECTIONS.render(out);
    LLM_CALLS.render(out);
    LLM_BREAKER_OPEN.render(out);
    CLUSTER_LAPSES.render(out);
    MESH_DROPPED.render(out);
    MESH_RESYNCS.render(out);
    MESH_LINKS.render(out);
    let _ = writeln!(out, "# TYPE harvest_turn_duration_milliseconds histogram");
    TURN_DURATION.render(out, "harvest_turn_duration_milliseconds", "");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn histograms_render_cumulative_buckets() {
        let histogram = Histogram::new(&[10, 100]);
        histogram.record(5);
        histogram.record(50);
        histogram.record(500);
        let mut out = String::new();
        histogram.render(&mut out, "h", "stage=\"x\"");
        assert!(out.contains("h_bucket{stage=\"x\",le=\"10\"} 1"));
        assert!(out.contains("h_bucket{stage=\"x\",le=\"100\"} 2"));
        assert!(out.contains("h_bucket{stage=\"x\",le=\"+Inf\"} 3"));
        assert!(out.contains("h_sum{stage=\"x\"} 555"));
        assert!(out.contains("h_count{stage=\"x\"} 3"));
    }

    #[test]
    fn counters_and_gauges_keep_one_series_per_label_set() {
        let counter = CounterVec::new("c");
        counter.inc(&[("outcome", "ok")]);
        counter.add(&[("outcome", "ok")], 2);
        counter.inc(&[("outcome", "error")]);
        assert_eq!(counter.get(&[("outcome", "ok")]), 3);
        let gauge = GaugeVec::new("g");
        gauge.set(&[("peer", "a")], 1);
        gauge.set(&[("peer", "a")], 0);
        let mut out = String::new();
        counter.render(&mut out);
        gauge.render(&mut out);
        assert!(out.contains("c{outcome=\"ok\"} 3"));
        assert!(out.contains("c{outcome=\"error\"} 1"));
        assert!(out.contains("g{peer=\"a\"} 0"));
    }

    #[test]
    fn label_values_are_escaped() {
        assert_eq!(label_set(&[("k", "a\"b")]), "k=\"a\\\"b\"");
    }
}
