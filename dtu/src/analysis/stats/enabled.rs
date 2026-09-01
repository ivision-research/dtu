use std::sync::atomic::{AtomicUsize, Ordering};

pub struct YokeCacheStats {
    attempts: AtomicUsize,
    lru_hits: AtomicUsize,
    db_hits: AtomicUsize,
    max_mem: AtomicUsize,
    memory_used: AtomicUsize,
}

impl YokeCacheStats {
    pub fn new() -> Self {
        Self {
            attempts: AtomicUsize::new(0),
            lru_hits: AtomicUsize::new(0),
            db_hits: AtomicUsize::new(0),
            memory_used: AtomicUsize::new(0),
            max_mem: AtomicUsize::new(0),
        }
    }
    pub fn lookup_attempt(&self) {
        self.attempts.fetch_add(1, Ordering::Relaxed);
    }
    pub fn lru_hit(&self) {
        self.lru_hits.fetch_add(1, Ordering::Relaxed);
    }
    pub fn db_hit(&self) {
        self.db_hits.fetch_add(1, Ordering::Relaxed);
    }
    pub fn add_cart_size(&self, size: usize) {
        self.memory_used.fetch_add(size, Ordering::Relaxed);
        self.max_mem.fetch_max(size, Ordering::Relaxed);
    }
}

impl Drop for YokeCacheStats {
    fn drop(&mut self) {
        let attempts = self.attempts.load(Ordering::Relaxed);
        let lru_hits = self.lru_hits.load(Ordering::Relaxed);
        let db_hits = self.db_hits.load(Ordering::Relaxed);

        eprintln!("YokeCacheStats stats:");
        eprintln!("\tLookups: {}", attempts);

        let lru_pct = 100.0f32 * (lru_hits as f32) / (attempts as f32);
        eprintln!("\tLRU hits: {} ({:.2}%)", lru_hits, lru_pct);
        let db_pct = 100.0f32 * (db_hits as f32) / (attempts as f32);
        eprintln!("\tDB hits: {} ({:.2}%)", db_hits, db_pct);

        let full_parses = attempts - lru_hits - db_hits;
        eprintln!(
            "\tFull parses: {} ({:.2}%)",
            full_parses,
            100.0f32 - lru_pct - db_pct
        );

        let total_mem = self.memory_used.load(Ordering::Relaxed) as f32;
        let max_mem = self.max_mem.load(Ordering::Relaxed) as f32;
        let total_mem_mb = total_mem / 1024f32 / 1024f32;
        let max_mem_kb = max_mem / 1024f32;
        let average_mem_kb = total_mem / 1024f32 / ((attempts - lru_hits) as f32);

        eprintln!("\tTotal memory: {total_mem_mb:.2}MiB");
        eprintln!("\tMax memory: {max_mem_kb:.2}KiB");
        eprintln!("\tAverage memory: {average_mem_kb:.2}KiB");
    }
}

pub struct CacheStats {
    attempts: usize,
    lru_hits: usize,
}

impl CacheStats {
    pub fn new() -> Self {
        Self {
            attempts: 0,
            lru_hits: 0,
        }
    }
    pub fn lookup_attempt(&mut self) {
        self.attempts += 1;
    }
    pub fn lru_hit(&mut self) {
        self.lru_hits += 1;
    }
}

impl Drop for CacheStats {
    fn drop(&mut self) {
        let attempts = self.attempts;
        let lru_hits = self.lru_hits;

        eprintln!("CacheStats stats:");
        eprintln!("\tLookups: {}", attempts);
        let lru_pct = 100.0f32 * (lru_hits as f32) / (attempts as f32);
        eprintln!("\tLRU hits: {} ({:.2}%)", lru_hits, lru_pct);
        let misses = attempts - lru_hits;
        eprintln!("\tMisses: {} ({:.2}%)", misses, 100.0f32 - lru_pct);
    }
}
