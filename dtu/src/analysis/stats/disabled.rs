pub struct YokeCacheStats;

impl YokeCacheStats {
    pub fn new() -> Self {
        Self
    }
    pub fn lookup_attempt(&self) {}
    pub fn lru_hit(&self) {}
    pub fn db_hit(&self) {}
    pub fn add_cart_size(&self, _size: usize) {}
}

pub struct CacheStats;

impl CacheStats {
    pub fn new(_name: &'static str) -> Self {
        Self
    }
    pub fn lookup_attempt(&self) {}
    pub fn lru_hit(&self) {}
}
