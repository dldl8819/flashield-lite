use crate::flash::SimulatedFlash;
use crate::lru::{CacheValue, LruCache};
use crate::metrics::Metrics;
use crate::trace::{Operation, TraceEvent};

pub trait CachePolicy {
    fn handle(&mut self, event: &TraceEvent, metrics: &mut Metrics);
    fn finish(&mut self, metrics: &mut Metrics);
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Object {
    size: u64,
}

impl Object {
    fn new(size: u64) -> Self {
        Self { size }
    }
}

impl CacheValue for Object {
    fn size(&self) -> u64 {
        self.size
    }
}

#[derive(Debug)]
pub struct DramLruCache {
    dram: LruCache<Object>,
}

impl DramLruCache {
    pub fn new(capacity_bytes: u64) -> Self {
        Self {
            dram: LruCache::new(capacity_bytes),
        }
    }
}

impl CachePolicy for DramLruCache {
    fn handle(&mut self, event: &TraceEvent, metrics: &mut Metrics) {
        metrics.record_request();
        match event.op {
            Operation::Get => {
                if self.dram.get(&event.key).is_some() {
                    metrics.record_dram_hit();
                } else {
                    metrics.record_miss();
                }
            }
            Operation::Set | Operation::Update => {
                metrics.evictions += self
                    .dram
                    .insert(event.key.clone(), Object::new(event.size))
                    .len() as u64;
            }
            Operation::Delete => {
                self.dram.remove(&event.key);
            }
        }
    }

    fn finish(&mut self, _metrics: &mut Metrics) {}
}

#[derive(Debug)]
pub struct NaiveFlashCache {
    dram: LruCache<Object>,
    flash: SimulatedFlash,
}

impl NaiveFlashCache {
    pub fn new(dram_capacity: u64, flash_capacity: u64, segment_size: u64) -> Self {
        Self {
            dram: LruCache::new(dram_capacity),
            flash: SimulatedFlash::new(flash_capacity, segment_size),
        }
    }
}

impl CachePolicy for NaiveFlashCache {
    fn handle(&mut self, event: &TraceEvent, metrics: &mut Metrics) {
        metrics.record_request();
        match event.op {
            Operation::Get => {
                if self.dram.get(&event.key).is_some() {
                    metrics.record_dram_hit();
                } else if let Some(object) = self.flash.get(&event.key) {
                    metrics.record_flash_hit();
                    metrics.evictions += self
                        .dram
                        .insert(event.key.clone(), Object::new(object.size()))
                        .len() as u64;
                } else {
                    metrics.record_miss();
                }
            }
            Operation::Set | Operation::Update => {
                metrics.evictions += self
                    .dram
                    .insert(event.key.clone(), Object::new(event.size))
                    .len() as u64;
                metrics.evictions += self.flash.write_object(event.key.clone(), event.size) as u64;
            }
            Operation::Delete => {
                self.dram.remove(&event.key);
                self.flash.delete(&event.key);
            }
        }
    }

    fn finish(&mut self, metrics: &mut Metrics) {
        self.flash.finish();
        metrics.absorb_flash_counters(
            self.flash.flash_bytes_written(),
            self.flash.logical_bytes_admitted(),
            self.flash.segment_flushes(),
        );
    }
}

#[derive(Debug, Clone, Copy)]
pub struct FlashieldConfig {
    pub dram_capacity: u64,
    pub flash_capacity: u64,
    pub segment_size: u64,
    pub min_reads: u64,
    pub max_updates: u64,
    pub min_age: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TrackedObject {
    size: u64,
    read_count: u64,
    update_count: u64,
    first_seen: u64,
    admitted: bool,
}

impl TrackedObject {
    fn new(size: u64, timestamp: u64, update_count: u64) -> Self {
        Self {
            size,
            read_count: 0,
            update_count,
            first_seen: timestamp,
            admitted: false,
        }
    }

    fn age_at(&self, timestamp: u64) -> u64 {
        timestamp.saturating_sub(self.first_seen)
    }

    fn eligible(&self, timestamp: u64, config: FlashieldConfig) -> bool {
        self.size > 0
            && !self.admitted
            && self.read_count >= config.min_reads
            && self.update_count <= config.max_updates
            && self.age_at(timestamp) >= config.min_age
    }
}

impl CacheValue for TrackedObject {
    fn size(&self) -> u64 {
        self.size
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MlConfig {
    pub flashield: FlashieldConfig,
    pub threshold: f64,
    pub learning_rate: f64,
}

const FEATURE_COUNT: usize = 5;

#[derive(Debug, Clone)]
struct FlashinessModel {
    weights: [f64; FEATURE_COUNT],
}

impl Default for FlashinessModel {
    fn default() -> Self {
        Self {
            weights: [-0.8, 2.4, 1.2, 0.1, -2.0],
        }
    }
}

impl FlashinessModel {
    fn score(&self, object: &TrackedObject, timestamp: u64) -> f64 {
        sigmoid(dot(self.weights, features(object, timestamp)))
    }

    fn train(&mut self, object: &TrackedObject, timestamp: u64, label: bool, learning_rate: f64) {
        let features = features(object, timestamp);
        let prediction = sigmoid(dot(self.weights, features));
        let target = if label { 1.0 } else { 0.0 };
        let error = target - prediction;

        for (weight, feature) in self.weights.iter_mut().zip(features) {
            *weight += learning_rate * error * feature;
        }
    }
}

fn features(object: &TrackedObject, timestamp: u64) -> [f64; FEATURE_COUNT] {
    [
        1.0,
        scaled_ln(object.read_count, 4.0),
        scaled_ln(object.age_at(timestamp), 6.0),
        scaled_ln(object.size, 12.0),
        scaled_ln(object.update_count, 4.0),
    ]
}

fn scaled_ln(value: u64, scale: f64) -> f64 {
    ((value as f64) + 1.0).ln() / scale
}

fn dot(weights: [f64; FEATURE_COUNT], features: [f64; FEATURE_COUNT]) -> f64 {
    weights
        .into_iter()
        .zip(features)
        .map(|(weight, feature)| weight * feature)
        .sum()
}

fn sigmoid(value: f64) -> f64 {
    1.0 / (1.0 + (-value).exp())
}

#[derive(Debug)]
pub struct FlashieldMlCache {
    config: MlConfig,
    dram: LruCache<TrackedObject>,
    flash: SimulatedFlash,
    model: FlashinessModel,
}

impl FlashieldMlCache {
    pub fn new(config: MlConfig) -> Self {
        Self {
            config,
            dram: LruCache::new(config.flashield.dram_capacity),
            flash: SimulatedFlash::new(
                config.flashield.flash_capacity,
                config.flashield.segment_size,
            ),
            model: FlashinessModel::default(),
        }
    }

    fn insert_tracked(
        &mut self,
        key: String,
        object: TrackedObject,
        timestamp: u64,
        metrics: &mut Metrics,
    ) {
        for (evicted_key, evicted_object) in self.dram.insert(key, object) {
            metrics.evictions += 1;
            self.try_admit(evicted_key, evicted_object.clone(), timestamp, metrics);
            self.train_from_outcome(&evicted_object, timestamp, false);
        }
    }

    fn try_admit(
        &mut self,
        key: String,
        mut object: TrackedObject,
        timestamp: u64,
        metrics: &mut Metrics,
    ) -> TrackedObject {
        if self.should_admit(&object, timestamp) {
            metrics.evictions += self.flash.write_object(key, object.size) as u64;
            object.admitted = true;
        }
        object
    }

    fn should_admit(&self, object: &TrackedObject, timestamp: u64) -> bool {
        object.size > 0
            && !object.admitted
            && object.read_count > 0
            && object.age_at(timestamp) >= self.config.flashield.min_age
            && self.model.score(object, timestamp) >= self.config.threshold
    }

    fn train_from_outcome(
        &mut self,
        object: &TrackedObject,
        timestamp: u64,
        ended_by_mutation: bool,
    ) {
        let label = !ended_by_mutation
            && object.read_count >= self.config.flashield.min_reads
            && object.update_count <= self.config.flashield.max_updates
            && object.age_at(timestamp) >= self.config.flashield.min_age;
        self.model
            .train(object, timestamp, label, self.config.learning_rate);
    }
}

impl CachePolicy for FlashieldMlCache {
    fn handle(&mut self, event: &TraceEvent, metrics: &mut Metrics) {
        metrics.record_request();
        match event.op {
            Operation::Get => {
                if let Some(mut object) = self.dram.get(&event.key) {
                    metrics.record_dram_hit();
                    object.read_count += 1;
                    let object =
                        self.try_admit(event.key.clone(), object, event.timestamp, metrics);
                    self.insert_tracked(event.key.clone(), object, event.timestamp, metrics);
                } else if let Some(object) = self.flash.get(&event.key) {
                    metrics.record_flash_hit();
                    let mut promoted = TrackedObject::new(object.size(), event.timestamp, 0);
                    promoted.read_count = self.config.flashield.min_reads;
                    promoted.admitted = true;
                    self.insert_tracked(event.key.clone(), promoted, event.timestamp, metrics);
                } else {
                    metrics.record_miss();
                }
            }
            Operation::Set => {
                self.flash.delete(&event.key);
                let existing = self.dram.remove(&event.key);
                let update_count = existing.as_ref().map_or(0, |object| object.update_count + 1);
                if let Some(object) = existing {
                    self.train_from_outcome(&object, event.timestamp, true);
                }
                let object = TrackedObject::new(event.size, event.timestamp, update_count);
                self.insert_tracked(event.key.clone(), object, event.timestamp, metrics);
            }
            Operation::Update => {
                self.flash.delete(&event.key);
                let mut object = if let Some(existing) = self.dram.remove(&event.key) {
                    self.train_from_outcome(&existing, event.timestamp, true);
                    existing
                } else {
                    TrackedObject::new(event.size, event.timestamp, 0)
                };
                object.size = event.size;
                object.update_count += 1;
                object.admitted = false;
                self.insert_tracked(event.key.clone(), object, event.timestamp, metrics);
            }
            Operation::Delete => {
                if let Some(object) = self.dram.remove(&event.key) {
                    self.train_from_outcome(&object, event.timestamp, true);
                }
                self.flash.delete(&event.key);
            }
        }
    }

    fn finish(&mut self, metrics: &mut Metrics) {
        self.flash.finish();
        metrics.absorb_flash_counters(
            self.flash.flash_bytes_written(),
            self.flash.logical_bytes_admitted(),
            self.flash.segment_flushes(),
        );
    }
}

#[derive(Debug)]
pub struct FlashieldLiteCache {
    config: FlashieldConfig,
    dram: LruCache<TrackedObject>,
    flash: SimulatedFlash,
}

impl FlashieldLiteCache {
    pub fn new(config: FlashieldConfig) -> Self {
        Self {
            config,
            dram: LruCache::new(config.dram_capacity),
            flash: SimulatedFlash::new(config.flash_capacity, config.segment_size),
        }
    }

    fn insert_tracked(
        &mut self,
        key: String,
        object: TrackedObject,
        timestamp: u64,
        metrics: &mut Metrics,
    ) {
        for (evicted_key, evicted_object) in self.dram.insert(key, object) {
            metrics.evictions += 1;
            self.try_admit(evicted_key, evicted_object, timestamp, metrics);
        }
    }

    fn try_admit(
        &mut self,
        key: String,
        mut object: TrackedObject,
        timestamp: u64,
        metrics: &mut Metrics,
    ) -> TrackedObject {
        if object.eligible(timestamp, self.config) {
            metrics.evictions += self.flash.write_object(key, object.size) as u64;
            object.admitted = true;
        }
        object
    }
}

impl CachePolicy for FlashieldLiteCache {
    fn handle(&mut self, event: &TraceEvent, metrics: &mut Metrics) {
        metrics.record_request();
        match event.op {
            Operation::Get => {
                if let Some(mut object) = self.dram.get(&event.key) {
                    metrics.record_dram_hit();
                    object.read_count += 1;
                    let object =
                        self.try_admit(event.key.clone(), object, event.timestamp, metrics);
                    self.insert_tracked(event.key.clone(), object, event.timestamp, metrics);
                } else if let Some(object) = self.flash.get(&event.key) {
                    metrics.record_flash_hit();
                    let mut promoted = TrackedObject::new(object.size(), event.timestamp, 0);
                    promoted.read_count = self.config.min_reads;
                    promoted.admitted = true;
                    self.insert_tracked(event.key.clone(), promoted, event.timestamp, metrics);
                } else {
                    metrics.record_miss();
                }
            }
            Operation::Set => {
                self.flash.delete(&event.key);
                let update_count = self
                    .dram
                    .remove(&event.key)
                    .map_or(0, |object| object.update_count + 1);
                let object = TrackedObject::new(event.size, event.timestamp, update_count);
                self.insert_tracked(event.key.clone(), object, event.timestamp, metrics);
            }
            Operation::Update => {
                self.flash.delete(&event.key);
                let mut object = self
                    .dram
                    .remove(&event.key)
                    .unwrap_or_else(|| TrackedObject::new(event.size, event.timestamp, 0));
                object.size = event.size;
                object.update_count += 1;
                object.admitted = false;
                self.insert_tracked(event.key.clone(), object, event.timestamp, metrics);
            }
            Operation::Delete => {
                self.dram.remove(&event.key);
                self.flash.delete(&event.key);
            }
        }
    }

    fn finish(&mut self, metrics: &mut Metrics) {
        self.flash.finish();
        metrics.absorb_flash_counters(
            self.flash.flash_bytes_written(),
            self.flash.logical_bytes_admitted(),
            self.flash.segment_flushes(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CachePolicy, FlashieldConfig, FlashieldLiteCache, FlashieldMlCache, FlashinessModel,
        MlConfig, NaiveFlashCache, TrackedObject,
    };
    use crate::metrics::Metrics;
    use crate::trace::{Operation, TraceEvent};

    fn trace_event(timestamp: u64, op: Operation, key: &str, size: u64) -> TraceEvent {
        TraceEvent {
            timestamp,
            op,
            key: key.to_string(),
            size,
        }
    }

    const DEFAULT_TEST_SEGMENT: u64 = 1;

    #[test]
    fn naive_flash_writes_every_set_and_update() {
        let events = [
            trace_event(1, Operation::Set, "synthetic:1", 100),
            trace_event(2, Operation::Update, "synthetic:1", 50),
        ];
        let mut cache = NaiveFlashCache::new(1024, 1024, DEFAULT_TEST_SEGMENT);
        let mut metrics = Metrics::default();

        for event in &events {
            cache.handle(event, &mut metrics);
        }
        cache.finish(&mut metrics);

        assert_eq!(metrics.logical_bytes_admitted, 150);
        assert_eq!(metrics.flash_bytes_written, 150);
    }

    #[test]
    fn flashield_lite_admits_stable_read_worthy_objects() {
        let config = FlashieldConfig {
            dram_capacity: 1024,
            flash_capacity: 1024,
            segment_size: DEFAULT_TEST_SEGMENT,
            min_reads: 2,
            max_updates: 0,
            min_age: 2,
        };
        let events = [
            trace_event(1, Operation::Set, "synthetic:1", 128),
            trace_event(2, Operation::Get, "synthetic:1", 0),
            trace_event(3, Operation::Get, "synthetic:1", 0),
        ];
        let mut cache = FlashieldLiteCache::new(config);
        let mut metrics = Metrics::default();

        for event in &events {
            cache.handle(event, &mut metrics);
        }
        cache.finish(&mut metrics);

        assert_eq!(metrics.logical_bytes_admitted, 128);
        assert_eq!(metrics.flash_bytes_written, 128);
    }

    #[test]
    fn update_heavy_workload_writes_less_with_flashield_lite() {
        let mut events = vec![trace_event(1, Operation::Set, "synthetic:1", 128)];
        for timestamp in 2..=40 {
            let op = if timestamp % 2 == 0 {
                Operation::Update
            } else {
                Operation::Get
            };
            events.push(trace_event(timestamp, op, "synthetic:1", 128));
        }

        let mut naive = NaiveFlashCache::new(1024, 4096, DEFAULT_TEST_SEGMENT);
        let mut naive_metrics = Metrics::default();
        for event in &events {
            naive.handle(event, &mut naive_metrics);
        }
        naive.finish(&mut naive_metrics);

        let config = FlashieldConfig {
            dram_capacity: 1024,
            flash_capacity: 4096,
            segment_size: DEFAULT_TEST_SEGMENT,
            min_reads: 2,
            max_updates: 1,
            min_age: 2,
        };
        let mut flashield = FlashieldLiteCache::new(config);
        let mut flashield_metrics = Metrics::default();
        for event in &events {
            flashield.handle(event, &mut flashield_metrics);
        }
        flashield.finish(&mut flashield_metrics);

        assert!(flashield_metrics.flash_bytes_written < naive_metrics.flash_bytes_written);
    }

    #[test]
    fn read_heavy_stable_workload_keeps_reasonable_hit_rate() {
        let config = FlashieldConfig {
            dram_capacity: 128,
            flash_capacity: 4096,
            segment_size: DEFAULT_TEST_SEGMENT,
            min_reads: 2,
            max_updates: 0,
            min_age: 1,
        };
        let mut events = Vec::new();
        let mut timestamp = 1;
        for key in ["synthetic:1", "synthetic:2", "synthetic:3", "synthetic:4"] {
            events.push(trace_event(timestamp, Operation::Set, key, 64));
            timestamp += 1;
            events.push(trace_event(timestamp, Operation::Get, key, 0));
            timestamp += 1;
            events.push(trace_event(timestamp, Operation::Get, key, 0));
            timestamp += 1;
        }
        for _ in 0..5 {
            for key in ["synthetic:1", "synthetic:2", "synthetic:3", "synthetic:4"] {
                events.push(trace_event(timestamp, Operation::Get, key, 0));
                timestamp += 1;
            }
        }

        let mut cache = FlashieldLiteCache::new(config);
        let mut metrics = Metrics::default();
        for event in &events {
            cache.handle(event, &mut metrics);
        }
        cache.finish(&mut metrics);

        assert!(metrics.hit_rate() >= 0.75);
        assert!(metrics.flash_hits > 0);
    }

    #[test]
    fn flashiness_model_learns_from_positive_examples() {
        let mut model = FlashinessModel::default();
        let mut object = TrackedObject::new(128, 1, 0);
        object.read_count = 3;

        let before = model.score(&object, 5);
        model.train(&object, 5, true, 0.5);
        let after = model.score(&object, 5);

        assert!(after > before);
    }

    #[test]
    fn flashield_ml_admits_stable_read_worthy_objects() {
        let config = MlConfig {
            flashield: FlashieldConfig {
                dram_capacity: 1024,
                flash_capacity: 1024,
                segment_size: DEFAULT_TEST_SEGMENT,
                min_reads: 2,
                max_updates: 0,
                min_age: 2,
            },
            threshold: 0.5,
            learning_rate: 0.1,
        };
        let events = [
            trace_event(1, Operation::Set, "synthetic:1", 128),
            trace_event(2, Operation::Get, "synthetic:1", 0),
            trace_event(3, Operation::Get, "synthetic:1", 0),
        ];
        let mut cache = FlashieldMlCache::new(config);
        let mut metrics = Metrics::default();

        for event in &events {
            cache.handle(event, &mut metrics);
        }
        cache.finish(&mut metrics);

        assert_eq!(metrics.logical_bytes_admitted, 128);
        assert_eq!(metrics.flash_bytes_written, 128);
    }

    #[test]
    fn update_heavy_workload_writes_less_with_flashield_ml_than_naive_flash() {
        let mut events = vec![trace_event(1, Operation::Set, "synthetic:1", 128)];
        for timestamp in 2..=40 {
            let op = if timestamp % 2 == 0 {
                Operation::Update
            } else {
                Operation::Get
            };
            events.push(trace_event(timestamp, op, "synthetic:1", 128));
        }

        let mut naive = NaiveFlashCache::new(1024, 4096, DEFAULT_TEST_SEGMENT);
        let mut naive_metrics = Metrics::default();
        for event in &events {
            naive.handle(event, &mut naive_metrics);
        }
        naive.finish(&mut naive_metrics);

        let config = MlConfig {
            flashield: FlashieldConfig {
                dram_capacity: 1024,
                flash_capacity: 4096,
                segment_size: DEFAULT_TEST_SEGMENT,
                min_reads: 2,
                max_updates: 1,
                min_age: 2,
            },
            threshold: 0.5,
            learning_rate: 0.1,
        };
        let mut flashield = FlashieldMlCache::new(config);
        let mut flashield_metrics = Metrics::default();
        for event in &events {
            flashield.handle(event, &mut flashield_metrics);
        }
        flashield.finish(&mut flashield_metrics);

        assert!(flashield_metrics.flash_bytes_written < naive_metrics.flash_bytes_written);
    }
}
