use crate::lru::{CacheValue, LruCache};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlashObject {
    size: u64,
}

impl FlashObject {
    pub fn new(size: u64) -> Self {
        Self { size }
    }
}

impl CacheValue for FlashObject {
    fn size(&self) -> u64 {
        self.size
    }
}

#[derive(Debug, Clone)]
pub struct SimulatedFlash {
    index: LruCache<FlashObject>,
    segment_size: u64,
    buffered_bytes: u64,
    flash_bytes_written: u64,
    logical_bytes_admitted: u64,
    segment_flushes: u64,
    finalized: bool,
}

impl SimulatedFlash {
    pub fn new(capacity_bytes: u64, segment_size: u64) -> Self {
        assert!(segment_size > 0, "segment_size must be non-zero");
        Self {
            index: LruCache::new(capacity_bytes),
            segment_size,
            buffered_bytes: 0,
            flash_bytes_written: 0,
            logical_bytes_admitted: 0,
            segment_flushes: 0,
            finalized: false,
        }
    }

    pub fn get(&mut self, key: &str) -> Option<FlashObject> {
        self.index.get(key)
    }

    pub fn write_object(&mut self, key: String, size: u64) -> usize {
        if size == 0 {
            return 0;
        }
        self.finalized = false;
        self.logical_bytes_admitted += size;
        self.buffered_bytes += size;

        while self.buffered_bytes >= self.segment_size {
            self.buffered_bytes -= self.segment_size;
            self.flash_bytes_written += self.segment_size;
            self.segment_flushes += 1;
        }

        self.index.insert(key, FlashObject::new(size)).len()
    }

    pub fn delete(&mut self, key: &str) {
        self.index.remove(key);
    }

    pub fn finish(&mut self) {
        if self.finalized {
            return;
        }

        if self.buffered_bytes > 0 {
            self.flash_bytes_written += self.segment_size;
            self.segment_flushes += 1;
            self.buffered_bytes = 0;
        }

        self.finalized = true;
    }

    pub fn flash_bytes_written(&self) -> u64 {
        self.flash_bytes_written
    }

    pub fn logical_bytes_admitted(&self) -> u64 {
        self.logical_bytes_admitted
    }

    pub fn segment_flushes(&self) -> u64 {
        self.segment_flushes
    }
}

#[cfg(test)]
mod tests {
    use super::SimulatedFlash;

    #[test]
    fn accounts_for_logical_and_physical_writes() {
        let mut flash = SimulatedFlash::new(1024, 64);

        assert_eq!(flash.write_object("a".to_string(), 40), 0);
        assert_eq!(flash.write_object("b".to_string(), 40), 0);
        flash.finish();

        assert_eq!(flash.logical_bytes_admitted(), 80);
        assert_eq!(flash.flash_bytes_written(), 128);
        assert_eq!(flash.segment_flushes(), 2);
    }
}
