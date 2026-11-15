use anyhow::anyhow;
use anyhow::{Error, Result};
use async_channel::{self, Receiver, Sender};
use bytes::Bytes;
use derivative::Derivative;
use futures::FutureExt as _;
use futures::{future::BoxFuture, Future};
use std::any::{Any, TypeId};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{cell::RefCell, collections::HashMap, hash::Hash, rc::Rc, sync::Arc};

use crate::image_cache::ImageCache;
use crate::{r#async::executor, Entity, ModelContext, SingletonEntity};

use super::AssetProvider;

static NEXT_ASSET_ACCESS_TICK: AtomicU64 = AtomicU64::new(1);

fn next_asset_access_tick() -> u64 {
    NEXT_ASSET_ACCESS_TICK.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LocalFileStamp {
    modified: Option<std::time::SystemTime>,
    len: u64,
}

#[cfg(not(target_family = "wasm"))]
fn stat_local_file(path: &str) -> Option<LocalFileStamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some(LocalFileStamp {
        modified: meta.modified().ok(),
        len: meta.len(),
    })
}

#[cfg(target_family = "wasm")]
fn stat_local_file(_path: &str) -> Option<LocalFileStamp> {
    None
}

pub trait FetchAsset: crate::r#async::Spawnable + Future<Output = Result<Bytes>> {}
impl<T: crate::r#async::Spawnable + Future<Output = Result<Bytes>> + ?Sized> FetchAsset for T {}

/// Marker trait for async asset ID namespaces.
///
/// Each distinct kind of async asset source defines its own zero-sized marker
/// type that implements this trait. The marker's [`TypeId`] is stored inside
/// [`AsyncAssetId`] so that IDs from different sources can never collide, even
/// if they happen to share the same key string.
pub trait AsyncAssetType: 'static {}

/// A namespaced identifier for an [`AssetSource::Async`] entry.
///
/// The namespace is stored as a [`TypeId`] derived from a marker type that
/// implements [`AsyncAssetType`]. This guarantees that two different async
/// sources cannot accidentally produce colliding cache keys.
#[derive(Clone, Hash, PartialEq, Eq)]
pub struct AsyncAssetId {
    namespace: TypeId,
    key: String,
}

impl AsyncAssetId {
    /// Creates a new ID in the namespace defined by `N`.
    pub fn new<N: AsyncAssetType>(key: impl Into<String>) -> Self {
        Self {
            namespace: TypeId::of::<N>(),
            key: key.into(),
        }
    }

    /// Returns the key portion of this ID.
    pub fn key(&self) -> &str {
        &self.key
    }
}

impl std::fmt::Debug for AsyncAssetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TypeId's Debug output is opaque, so just print the key.
        f.debug_struct("AsyncAssetId")
            .field("key", &self.key)
            .finish()
    }
}

/// A "URI" for some data file. In other words, the location of an asset.
#[derive(Derivative)]
#[derivative(Clone, Hash, PartialEq, Eq, Debug)]
pub enum AssetSource {
    /// Loaded from an arbitrary asynchronous source (e.g. a URL fetch).
    Async {
        /// A namespaced identifier used as the cache key.
        id: AsyncAssetId,
        /// A factory that produces the future to fetch the asset bytes.
        /// Called at most once per unique `id` — only when the asset is
        /// not already loaded or loading.
        #[derivative(Hash = "ignore", PartialEq = "ignore", Debug = "ignore")]
        fetch: Arc<dyn Fn() -> Pin<Box<dyn FetchAsset>> + Send + Sync>,
    },
    /// Included in the app bundle.
    Bundled {
        // Assets that are statically included in the bundle can be statically
        // referenced, hence using a `&'static str` here and not a `String`.
        path: &'static str,
    },
    /// Accessible in the user's local filesystem at the provided path.
    LocalFile { path: String },
    /// Image loaded directly with bytes
    Raw { id: String },
}

/// The public representation of an asset's current state (i.e., in-memory availability).
pub enum AssetState<T> {
    Loading { handle: AssetHandle },
    Loaded { data: Rc<T> },
    Evicted,
    FailedToLoad(Rc<Error>),
}

/// An external type so views can refer to the asset they requested.
/// Transforms into a future that resolves once the asset is finished loading, allowing
/// work to be scheduled at the time of load completion.
#[derive(Clone, Hash, PartialEq, Eq, Debug)]
pub struct AssetHandle {
    source: AssetSource,
    asset_type: TypeId,
}

impl AssetHandle {
    /// Creates a future that resolves whenever the asset is finished loading.
    pub fn when_loaded(&self, asset_cache: &AssetCache) -> Option<BoxFuture<'static, ()>> {
        asset_cache.create_future_for_loading_asset(self)
    }
}

/// An internal representation of an asset's state, as it's tracked and updated by the
/// AssetCache. An implementation.
enum AssetStateInternal {
    Loading {
        channel: (Sender<()>, Receiver<()>),
    },
    Loaded {
        data: Rc<dyn Any>,
        timestamp: u64,
        size_in_bytes: usize,
        local_file_stamp: Option<LocalFileStamp>,
    },
    Evicted,
    Error {
        err: Rc<Error>,
        local_file_stamp: Option<LocalFileStamp>,
    },
}

impl AssetStateInternal {
    fn loading() -> Self {
        // Whenever we add an asset in a loading state, we create a channel that
        // can be alerted once the asset load completes (i.e., becomes available
        // or encounters an error). The channel must support the ability to clone one
        // side of the channel.
        let channel = async_channel::bounded(1);
        AssetStateInternal::Loading { channel }
    }

    fn to_external_type<T: Asset>(&self, source: AssetSource) -> AssetState<T> {
        match self {
            AssetStateInternal::Loading { .. } => AssetState::Loading {
                handle: AssetHandle {
                    source,
                    asset_type: TypeId::of::<T>(),
                },
            },
            AssetStateInternal::Loaded { data, .. } => AssetState::Loaded {
                data: data
                    .clone()
                    .downcast::<T>()
                    .expect("should not fail to downcast"),
            },
            AssetStateInternal::Evicted => AssetState::Evicted,
            AssetStateInternal::Error { err, .. } => AssetState::FailedToLoad(err.clone()),
        }
    }
}

/// A general-purpose data cache for managing assets. Generalized to any file type.
/// Internally handles networking and persistence caching.
pub struct AssetCache {
    // Note: interior mutability allows us to update the state of an asset
    // without requiring a mutable reference to the AssetCache.
    inner: Rc<RefCell<HashMap<AssetHandle, AssetStateInternal>>>,
    max_cached_asset_bytes: usize,

    bundled_asset_provider: Box<dyn AssetProvider>,
    foreground_executor: Rc<executor::Foreground>,
    background_executor: Arc<executor::Background>,
}

pub trait Asset: Any {
    fn try_from_bytes(data: &[u8]) -> anyhow::Result<Self>
    where
        Self: Sized;

    fn size_in_bytes(&self) -> usize;
}

impl Asset for String {
    fn try_from_bytes(data: &[u8]) -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        std::str::from_utf8(data)
            .map(|s| s.to_string())
            .map_err(|e| e.into())
    }

    fn size_in_bytes(&self) -> usize {
        self.len()
    }
}

fn is_evictable_source(source: &AssetSource) -> bool {
    matches!(
        source,
        AssetSource::Raw { .. } | AssetSource::LocalFile { .. } | AssetSource::Async { .. }
    )
}

fn evict_lru_assets_map(
    assets: &mut HashMap<AssetHandle, AssetStateInternal>,
    max_bytes: usize,
    mut on_evict: impl FnMut(&AssetSource),
) -> Vec<u32> {
    let mut total_size: usize = assets
        .iter()
        .filter_map(|(handle, state)| {
            if is_evictable_source(&handle.source) {
                if let AssetStateInternal::Loaded { size_in_bytes, .. } = state {
                    return Some(*size_in_bytes);
                }
            }
            None
        })
        .fold(0usize, usize::saturating_add);

    if total_size <= max_bytes {
        return vec![];
    }

    let mut evictable_assets: Vec<_> = assets
        .iter()
        .filter_map(|(handle, state)| {
            if is_evictable_source(&handle.source) {
                if let AssetStateInternal::Loaded {
                    timestamp,
                    size_in_bytes,
                    ..
                } = state
                {
                    return Some((handle.clone(), *timestamp, *size_in_bytes));
                }
            }
            None
        })
        .collect();

    // Sort by timestamp ascending (least recently used first).
    evictable_assets.sort_by_key(|&(_, timestamp, _)| timestamp);

    let mut evicted_image_ids = vec![];
    let total_candidates = evictable_assets.len();

    for (idx, (handle, _, size_in_bytes)) in evictable_assets.into_iter().enumerate() {
        if total_size <= max_bytes {
            break;
        }
        // Keep the single most-recently-loaded entry if it alone exceeds a tiny test cap,
        // unless it is a Raw asset exceeding MAX_CACHED_ASSET_BYTES.
        if idx + 1 == total_candidates
            && !matches!(handle.source, AssetSource::Raw { .. })
            && total_candidates == 1
        {
            break;
        }

        if assets.remove(&handle).is_some() {
            on_evict(&handle.source);
            total_size = total_size.saturating_sub(size_in_bytes);

            if let AssetSource::Raw { id } = &handle.source {
                assets.insert(handle.clone(), AssetStateInternal::Evicted);
                if let Ok(id) = id.parse::<u32>() {
                    evicted_image_ids.push(id);
                }
            }
        }
    }

    evicted_image_ids
}

impl AssetCache {
    const MAX_RAW_ASSET_SIZE: usize = 320 * 1024 * 1024; // 320 MiB

    pub fn new(
        bundled_asset_provider: Box<dyn AssetProvider>,
        foreground_executor: Rc<executor::Foreground>,
        background_executor: Arc<executor::Background>,
    ) -> Self {
        Self {
            inner: Rc::new(RefCell::new(HashMap::new())),
            max_cached_asset_bytes: Self::MAX_RAW_ASSET_SIZE,
            bundled_asset_provider,
            foreground_executor,
            background_executor,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_max_cached_bytes(
        bundled_asset_provider: Box<dyn AssetProvider>,
        foreground_executor: Rc<executor::Foreground>,
        background_executor: Arc<executor::Background>,
        max_cached_asset_bytes: usize,
    ) -> Self {
        Self {
            inner: Rc::new(RefCell::new(HashMap::new())),
            max_cached_asset_bytes,
            bundled_asset_provider,
            foreground_executor,
            background_executor,
        }
    }

    /// Tracks the current total size of raw assets in memory.
    pub fn get_total_raw_asset_size(&self) -> usize {
        self.inner
            .borrow()
            .iter()
            .filter_map(|(handle, state)| {
                if let AssetStateInternal::Loaded { size_in_bytes, .. } = state {
                    if matches!(handle.source, AssetSource::Raw { .. }) {
                        return Some(*size_in_bytes);
                    }
                }
                None
            })
            .fold(0usize, usize::saturating_add)
    }

    /// Tracks the current total size of all evictable (`Raw`, `LocalFile`, `Async`) assets in memory.
    pub fn get_total_evictable_asset_size(&self) -> usize {
        self.inner
            .borrow()
            .iter()
            .filter_map(|(handle, state)| {
                if is_evictable_source(&handle.source) {
                    if let AssetStateInternal::Loaded { size_in_bytes, .. } = state {
                        return Some(*size_in_bytes);
                    }
                }
                None
            })
            .fold(0usize, usize::saturating_add)
    }

    /// Removes the least recently used evictable assets until the total size is within the limit.
    fn evict_raw_assets_if_needed(&self, ctx: &ModelContext<Self>) -> Vec<u32> {
        let mut assets = self.inner.borrow_mut();
        evict_lru_assets_map(&mut assets, self.max_cached_asset_bytes, |source| {
            ImageCache::as_ref(ctx).evict_image(source);
        })
    }

    /// The main API of the asset cache. Given the location of an asset, returns an indicator of the
    /// in-memory availability of the asset. If the asset is not already loaded or loading, a background
    /// task is spawned to perform the retrieval.
    ///
    /// Note: this is an idempotent operation. It can be called as many times as needed on a given
    /// asset and won't duplicate work.
    pub fn load_asset<T: Asset>(&self, source: AssetSource) -> AssetState<T> {
        let mut assets = self.inner.borrow_mut();

        let key = AssetHandle {
            source: source.clone(),
            asset_type: TypeId::of::<T>(),
        };

        // For LocalFile assets, verify `(mtime, len)` so overwritten or newly-created local
        // files automatically reload instead of serving stale cached pixels.
        if let AssetSource::LocalFile { ref path } = source {
            let should_reload = match assets.get(&key) {
                Some(AssetStateInternal::Loaded {
                    local_file_stamp, ..
                }) => stat_local_file(path) != *local_file_stamp,
                Some(AssetStateInternal::Error {
                    local_file_stamp, ..
                }) => stat_local_file(path) != *local_file_stamp,
                _ => false,
            };
            if should_reload {
                assets.remove(&key);
            }
        }

        // Update LRU access timestamp on cache hits.
        if let Some(AssetStateInternal::Loaded { timestamp, .. }) = assets.get_mut(&key) {
            *timestamp = next_asset_access_tick();
        }

        if !assets.contains_key(&key) {
            match source.clone() {
                AssetSource::Async { fetch, .. } => {
                    assets.insert(key.clone(), AssetStateInternal::loading());
                    let future = (fetch)();
                    self.load_asynchronously::<T>(source.clone(), None, future);
                }
                AssetSource::Bundled { path } => {
                    let asset_state = match self
                        .bundled_asset_provider
                        .get(path)
                        .and_then(|bytes| T::try_from_bytes(&bytes))
                    {
                        Ok(asset) => {
                            let timestamp = next_asset_access_tick();
                            let size_in_bytes = asset.size_in_bytes();

                            AssetStateInternal::Loaded {
                                data: Rc::new(asset) as Rc<dyn Any>,
                                timestamp,
                                size_in_bytes,
                                local_file_stamp: None,
                            }
                        }
                        Err(err) => AssetStateInternal::Error {
                            err: Rc::new(err),
                            local_file_stamp: None,
                        },
                    };
                    assets.insert(key.clone(), asset_state);
                }
                AssetSource::LocalFile { path } => {
                    let initial_stamp = stat_local_file(&path);
                    assets.insert(key.clone(), AssetStateInternal::loading());
                    self.load_local_file_asset::<T>(source.clone(), path, initial_stamp);
                }
                AssetSource::Raw { id } => {
                    assets.insert(
                        key.clone(),
                        AssetStateInternal::Error {
                            err: Rc::new(anyhow!("Raw image with ID {:?} did not exist", id)),
                            local_file_stamp: None,
                        },
                    );
                }
            };
        }

        assets[&key].to_external_type(source)
    }

    fn load_local_file_asset<T: Asset>(
        &self,
        source: AssetSource,
        path: String,
        initial_stamp: Option<LocalFileStamp>,
    ) {
        let read_path = path.clone();
        self.load_asynchronously::<T>(
            source,
            initial_stamp,
            Box::pin(async move {
                let buffer = async_fs::read(read_path).await?;
                Ok(buffer.into())
            }),
        );
    }

    pub fn insert_raw_asset_bytes<T: Asset>(
        &self,
        id: String,
        bytes: &[u8],
        ctx: &mut ModelContext<Self>,
    ) {
        let mut assets = self.inner.borrow_mut();
        let source = AssetSource::Raw { id: id.clone() };
        let key = AssetHandle {
            source: source.clone(),
            asset_type: TypeId::of::<T>(),
        };
        match T::try_from_bytes(bytes) {
            Ok(asset) => {
                let timestamp = next_asset_access_tick();
                let size_in_bytes = asset.size_in_bytes();

                assets.insert(
                    key.clone(),
                    AssetStateInternal::Loaded {
                        data: Rc::new(asset) as Rc<dyn Any>,
                        timestamp,
                        size_in_bytes,
                        local_file_stamp: None,
                    },
                );
            }
            Err(err) => {
                log::warn!("Raw asset conversion failed (ID: {id}): {err:#}");
                assets.insert(
                    key.clone(),
                    AssetStateInternal::Error {
                        err: Rc::new(err),
                        local_file_stamp: None,
                    },
                );
            }
        };

        ImageCache::as_ref(ctx).evict_image(&source);

        drop(assets);
        let image_ids = self.evict_raw_assets_if_needed(ctx);

        if !image_ids.is_empty() {
            ctx.emit(AssetCacheEvent::ImagesEvicted { image_ids });
        }
    }

    // Creates a future that resolves when an asset is loaded into moemory.
    fn create_future_for_loading_asset(
        &self,
        asset_handle: &AssetHandle,
    ) -> Option<BoxFuture<'static, ()>> {
        let assets = self.inner.borrow_mut();

        assets.get(asset_handle).map(|asset_state| {
            match asset_state {
                AssetStateInternal::Loading { channel } => {
                    // Internally, the future works by cloning a new receiver on the channel that's assigned
                    // to this asset. Inside the future, we simply wait on the receiving end of the channel.
                    // Note that the channel is held by the AssetStateInternal::Loading variant, so when the asset
                    // is promoted to the Loaded or FailedToLoad variants, the channel is dropped. This returns a
                    // RecvError to any receivers, serving as our notification that the asset is no longer loading.
                    let rx = channel.1.clone();
                    async move {
                        let _ = rx.recv().await;
                    }
                    .boxed()
                }
                // If the asset isn't currently loading, it is either already loaded or it's in an error state. Either
                // way, we should return a future that resolves immediately since there's no more pending updates
                // for this asset.
                _ => futures::future::ready(()).boxed(),
            }
        })
    }

    // Helper method to spawn the futures that perform an asset load and place the results into the asset cache.
    fn load_asynchronously<T: Asset>(
        &self,
        asset_source: AssetSource,
        initial_stamp: Option<LocalFileStamp>,
        future: Pin<Box<dyn FetchAsset>>,
    ) {
        let (tx, rx) = futures::channel::oneshot::channel();
        let max_cached_asset_bytes = self.max_cached_asset_bytes;

        // Spawn the work on the background executor.
        self.background_executor
            .spawn(async move {
                let result = future.await;
                // When the fetch finished, send the results to the future running on the foreground executor.
                if tx.send(result).is_err() {
                    log::error!("Error sending background task result to main thread");
                }
            })
            .detach();

        // Spawn a receiver on the foreground executor.
        let assets = Rc::downgrade(&self.inner);
        self.foreground_executor
            .spawn_boxed(Box::pin(async move {
                let result = match rx.await {
                    Ok(result) => result,
                    Err(_) => {
                        let msg = "sender unexpectedly dropped before receiver";
                        log::error!("{msg}");
                        Err(anyhow!(msg))
                    }
                };

                let Some(assets) = assets.upgrade() else {
                    return;
                };

                let mut assets = assets.borrow_mut();

                let local_file_stamp = match &asset_source {
                    AssetSource::LocalFile { path } => stat_local_file(path).or(initial_stamp),
                    _ => None,
                };

                // Populate the asset cache with the result.
                let handle = AssetHandle {
                    source: asset_source.clone(),
                    asset_type: TypeId::of::<T>(),
                };
                match result {
                    Ok(bytes) => match T::try_from_bytes(&bytes) {
                        Ok(asset) => {
                            log::debug!("Asset fetch succeeded: {asset_source:?}");

                            let timestamp = next_asset_access_tick();
                            let size_in_bytes = asset.size_in_bytes();

                            assets.insert(
                                handle,
                                AssetStateInternal::Loaded {
                                    data: Rc::new(asset) as Rc<dyn Any>,
                                    timestamp,
                                    size_in_bytes,
                                    local_file_stamp,
                                },
                            );
                            let _ = evict_lru_assets_map(&mut assets, max_cached_asset_bytes, |_| {});
                        }
                        Err(err) => {
                            log::warn!("Asset conversion failed ({asset_source:?}): {err:#}");
                            assets.insert(
                                handle,
                                AssetStateInternal::Error {
                                    err: Rc::new(err),
                                    local_file_stamp,
                                },
                            );
                        }
                    },
                    Err(err) => {
                        log::warn!("Asset fetch failed ({asset_source:?}): {err:#}");
                        assets.insert(
                            handle,
                            AssetStateInternal::Error {
                                err: Rc::new(err),
                                local_file_stamp,
                            },
                        );
                    }
                }
            }))
            .detach();
    }
}

#[derive(Debug, Clone)]
pub enum AssetCacheEvent {
    ImagesEvicted { image_ids: Vec<u32> },
}

impl Entity for AssetCache {
    type Event = AssetCacheEvent;
}

impl SingletonEntity for AssetCache {}
