#[cfg(feature = "lru_stats")]
pub mod enabled;

#[cfg(feature = "lru_stats")]
pub use enabled::ClassLoaderStats;

#[cfg(not(feature = "lru_stats"))]
pub mod disabled;

#[cfg(not(feature = "lru_stats"))]
pub use disabled::ClassLoaderStats;
