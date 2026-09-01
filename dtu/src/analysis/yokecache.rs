use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use diesel::prelude::*;
use diesel::r2d2::{self, ConnectionManager, CustomizeConnection, Pool};
use diesel::{connection::SimpleConnection, QueryDsl, SqliteConnection};
use serde::{Deserialize, Serialize};
use yoke::{Yoke, Yokeable};

use crate::db::{query, DatabaseId};
use crate::{analysis::YokeCacheStats, Context};

pub type Cart = Box<[u8]>;
pub type Yoked<V> = Yoke<V, Cart>;

/// A generic cache for data that can be serialized and yoked
///
/// This type holds an LRU cache as well as a database connection to a cache database. It will check
/// the LRU cache on all retrievals and fall back to the database if there is nothing in the cache.
pub struct YokeCache<K, V>
where
    for<'b> V: Yokeable<'b>,
{
    cache: Mutex<lru::LruCache<K, Arc<Yoked<V>>>>,
    pool: Pool<ConnectionManager<SqliteConnection>>,
    stats: YokeCacheStats,
}

#[derive(Debug)]
struct SqlitePragmas;

diesel::table! {
    serialized(id) {
        id -> Integer,
        bytes -> Binary
    }
}

impl CustomizeConnection<SqliteConnection, r2d2::Error> for SqlitePragmas {
    fn on_acquire(&self, conn: &mut SqliteConnection) -> Result<(), r2d2::Error> {
        // I don't know how create the table with the type created by `diesel::table`
        conn.batch_execute(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA foreign_keys = ON;

             CREATE TABLE IF NOT EXISTS serialized
             (
                id          INTEGER NOT NULL,
                bytes       BLOB NOT NULL,

                PRIMARY KEY (id)
            );",
        )
        .map_err(r2d2::Error::QueryError)
    }
}

/// Serialize the value to the serialization format used by the cache
pub fn serialize<T: Serialize>(it: &T) -> anyhow::Result<Vec<u8>> {
    Ok(postcard::to_stdvec(it)?)
}

/// Deserialize the byte format used by the cache
pub fn deserialize<'de, T, B>(data: &'de B) -> anyhow::Result<T>
where
    T: Deserialize<'de>,
    B: AsRef<[u8]> + ?Sized,
{
    Ok(postcard::from_bytes::<T>(data.as_ref())?)
}

impl<K, V> YokeCache<K, V>
where
    K: DatabaseId,
    for<'a> V: Yokeable<'a>,
    for<'de> <V as Yokeable<'de>>::Output: Deserialize<'de>,
    V: Serialize,
{
    pub fn new(ctx: &dyn Context, dbfile: &str) -> Option<Self> {
        let db_path = ctx.get_sqlite_dir().ok()?.join(dbfile);
        let url = format!("sqlite://{}", db_path.to_string_lossy());

        let pool = Pool::builder()
            .max_size(rayon::current_num_threads() as u32)
            .connection_customizer(Box::new(SqlitePragmas))
            .build(ConnectionManager::<SqliteConnection>::new(&url))
            .ok()?;

        Some(Self {
            pool,
            stats: YokeCacheStats::new(),
            cache: Mutex::new(lru::LruCache::new(NonZeroUsize::new(64).unwrap())),
        })
    }

    /// Attempt to get an [Arc<Yoked<V>>] from the cache, creating a new one if needed
    ///
    /// The create function must return the serialized bytes of the type using [serialize]!
    pub fn get<F>(&self, key: K, create: F) -> anyhow::Result<Arc<Yoked<V>>>
    where
        F: Fn() -> anyhow::Result<Vec<u8>>,
    {
        self.stats.lookup_attempt();
        if let Some(cached) = self.cache_get(key) {
            self.stats.lru_hit();
            return Ok(cached);
        }

        if let Some(value) = self.get_from_db(key).ok() {
            self.stats.db_hit();
            let value = Arc::new(value);
            self.cache_put(key, &value);
            return Ok(value);
        };

        let bytes = create()?;

        let value = <Yoked<V>>::try_attach_to_cart(
            bytes.into_boxed_slice(),
            |data| -> anyhow::Result<_> { deserialize(data) },
        )?;

        let bytes = value.backing_cart();
        // Failing to add it to the database is a bummer but not fatal
        self.try_add_to_database(key, &bytes);

        let value = Arc::new(value);
        self.cache_put(key, &value);
        Ok(value)
    }

    fn cache_get(&self, key: K) -> Option<Arc<Yoked<V>>> {
        let mut locked = self.cache.lock().expect("mutex poisoned");
        locked.get(&key).map(Arc::clone)
    }

    fn cache_put(&self, key: K, value: &Arc<Yoked<V>>) {
        let size = value.backing_cart().len();
        self.stats.add_cart_size(size);
        let mut locked = self.cache.lock().expect("mutex poisoned");
        locked.put(key, Arc::clone(value));
    }

    fn get_from_db(&self, key: K) -> anyhow::Result<Yoked<V>> {
        let mut conn = self.pool.get()?;

        let bytes = query!(serialized::table
            .filter(serialized::id.eq(key.id()))
            .select(serialized::bytes))
        .first::<Vec<u8>>(&mut conn)
        .map(|it| Vec::into_boxed_slice(it))?;

        <Yoked<V>>::try_attach_to_cart(bytes, |data| -> anyhow::Result<_> { deserialize(data) })
    }

    fn try_add_to_database(&self, key: K, bytes: &[u8]) {
        let Ok(mut conn) = self.pool.get() else {
            log::error!("failed to get an sqlite connection from the pool");
            return;
        };

        // Since the cache is generic and it may be used from multiple threads, it is possible that
        // `create` was called for the same key multiple times before being inserted into the
        // database. Losing the race on insertion here isn't an issue. In an ideal world we'd have
        // some way to prevent the double create calls, but so it goes.

        if let Err(e) = query!(diesel::insert_into(serialized::table)
            .values((serialized::id.eq(key.id()), serialized::bytes.eq(bytes),))
            .on_conflict_do_nothing())
        .execute(&mut conn)
        {
            log::error!("failed to add serialized value to the database: {}", e);
        }
    }
}
