#[derive(Debug, Default, Clone, PartialEq)]
pub struct Metrics {
    pub total_requests: u64,
    pub lookup_requests: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    pub dram_hits: u64,
    pub flash_hits: u64,
    pub flash_bytes_written: u64,
    pub logical_bytes_admitted: u64,
    pub segment_flushes: u64,
    pub evictions: u64,
}

impl Metrics {
    pub fn record_request(&mut self) {
        self.total_requests += 1;
    }

    pub fn record_dram_hit(&mut self) {
        self.lookup_requests += 1;
        self.cache_hits += 1;
        self.dram_hits += 1;
    }

    pub fn record_flash_hit(&mut self) {
        self.lookup_requests += 1;
        self.cache_hits += 1;
        self.flash_hits += 1;
    }

    pub fn record_miss(&mut self) {
        self.lookup_requests += 1;
        self.cache_misses += 1;
    }

    pub fn absorb_flash_counters(&mut self, flash_bytes: u64, logical_bytes: u64, flushes: u64) {
        self.flash_bytes_written = flash_bytes;
        self.logical_bytes_admitted = logical_bytes;
        self.segment_flushes = flushes;
    }

    pub fn hit_rate(&self) -> f64 {
        let lookups = self.cache_hits + self.cache_misses;
        if lookups == 0 {
            0.0
        } else {
            self.cache_hits as f64 / lookups as f64
        }
    }

    pub fn write_amplification(&self) -> f64 {
        if self.logical_bytes_admitted == 0 {
            0.0
        } else {
            self.flash_bytes_written as f64 / self.logical_bytes_admitted as f64
        }
    }
}
