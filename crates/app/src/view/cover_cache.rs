//! The two per-frame costs a cover has, and the cache that removes both.
//!
//! # What this exists to stop (PERF-01, PERF-02)
//!
//! Both costs come from the same place: a cover's *kind* and a cover's *bytes*
//! are pure functions of a file, and the view was asking for them again on
//! every frame.
//!
//! - **PERF-01.** [`CoverSource::classify`](super::cover::CoverSource::classify)
//!   does `Path::new(cover_path).is_file()` (a `statx`) and, when the file
//!   exists, reads four bytes through `File::open` (an `openat` + a `read`). It
//!   was called from `cover_plan` *and* again from `framed`, i.e. twice per
//!   rendered cover, and `view()` runs every frame. Measured by the audit over
//!   a 3.572 s window of 100 forced redraws with 333 covers on disk: 6,624
//!   cover-file syscalls, with **all 333 distinct cover paths touched every
//!   frame** in a 1200×800 window that shows a few dozen tiles
//!   (`docs/audit/PERFORMANCE.md`, PERF-01). The answer cannot have changed:
//!   it is the same defect class as the runner label (PERF-06) — a value that
//!   only changes when the library is re-read.
//! - **PERF-02.** The decoded pixmap iced retains is stored at the *source
//!   image's own* dimensions (`iced/tiny_skia/src/raster.rs:151-171`), and the
//!   trim that bounds that cache retains only what was drawn in the last frame
//!   (`:232-237`) — which, with every game drawn every frame, is the whole
//!   library. Measured: 753 MB of RSS for 333 covers at 600×900, against
//!   333 × 600 × 900 × 4 = 719 MB of decoded pixels. [`CoverCache`] hands iced
//!   pixels that this app decoded and bounded, and keeps those pixels under a
//!   byte budget of its own, so residency is bounded by *this* budget rather
//!   than by the library's size.
//!
//! # The decode cap is the reference's own number
//!
//! [`DECODE_MAX`] is 512, and it is not chosen here: `CoverArt.qml:68-69` sets
//! `sourceSize.width: 512` and `sourceSize.height: 512` on the one `Image` that
//! draws a cover, so the Python app decodes no cover larger than 512 in either
//! axis either. That is the parity argument *and* the size argument: the widest
//! box this app draws a cover in is a card's, 188×207
//! ([`metrics::card_cover_box`](super::metrics::card_cover_box)), so a 512-axis
//! decode is at least twice the drawn size in both axes and does not upscale on
//! a 2× display.
//!
//! # The one behavioural difference, stated
//!
//! iced's own path loader applies the EXIF orientation tag to a JPEG before it
//! hands the pixels on (`iced/graphics/src/image.rs`, the `Handle::Path` arm's
//! `Operation::from_exif(...).perform(image)`). Decoding here does not, so a
//! photograph whose EXIF says "rotate 90" is drawn as stored. This is a
//! divergence from *this port's previous behaviour*, not from the reference:
//! the QML's `Image` has `autoTransform` at its default of `false` and
//! `CoverArt.qml` never sets it, so the Python app draws that photograph the
//! same way this does now.
//!
//! # What it does not do
//!
//! It is not a filesystem watcher. An entry is keyed by the cover *path* and
//! holds the answer for the file at that path, so replacing the bytes at a path
//! that is already cached — or deleting the file — does not invalidate it on
//! its own. The app calls [`CoverCache::forget`] at every point where it knows a
//! cover was rewritten: `Message::CoverFileChosen`'s copy, both
//! `CoverFetchFinished` arms, and the easy-install icon extraction. An edit made
//! outside the app (a cover deleted in a file manager, say) is therefore not
//! seen until one of those happens or the app restarts — recorded here rather
//! than left for a reader to discover, and the reason the invalidation entry
//! points exist at all.
//!
//! [`CoverCache::clear`] is the other one, and it has **no caller in production
//! code today**: `State::new` loads the library once
//! (`crates/app/src/main.rs:186`) and nothing replaces `state.library` at
//! runtime, so there is no re-read to hang it off. It exists because that
//! re-read is the event it belongs to and the cache's own invariants are
//! incomplete without it, and it is exercised in tests. Written down rather than
//! left as an apparent gap: a reader who greps for its callers will find none,
//! and that is the current state of the app rather than an oversight.

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::VecDeque;

use cosmic::iced::widget::image::Handle;

use super::cover::CoverSource;

/// The largest a cover is decoded to, in either axis. `CoverArt.qml:68-69`.
///
/// The reference's own `sourceSize`, so this is a port of a number the Python
/// app already used rather than a new policy — see the module docs for the
/// other half of the argument (it is ≥2× the widest box the app draws).
pub const DECODE_MAX: u32 = 512;

/// How many bytes of decoded cover pixels this cache may hold.
///
/// A budget rather than a count, because that is the quantity that grows with
/// the *source* image and the one PERF-02 measured: iced's own trim bounds its
/// cache by "what was drawn in the last frame", which is the library, so the
/// bound has to live somewhere that knows what a byte of pixels costs.
///
/// The arithmetic: a cover at the cap is 512 × 512 × 4 = 1 MiB, and a typical
/// 2:3 cover is 341 × 512 × 4 = 700 KiB. The window PERF-03 builds draws **45
/// cards** in the grid and **36 rows** in the list at 1200×800, so one window at
/// the cap is 45 × 1 MiB = 45 MiB. 64 MiB holds that window with 1.4× to spare,
/// and about two windows at the typical 2:3 size — which is what makes a short
/// scroll back through tiles just seen free rather than a decode.
///
/// Stated as headroom rather than as a guarantee, because 1.4× is not much: a
/// window of *square, at-cap* covers nearly fills the budget on its own, and a
/// scroll longer than a screen re-decodes at the typical size too. The knob for
/// that is this constant.
///
/// The window is the load-bearing bound and this budget is the backstop — with
/// PERF-03's window in place, iced's own cache never sees more than a window
/// either. Both are kept because each bounds something the other does not: the
/// window bounds what is *asked for*, the budget bounds what is *held*.
pub const IMAGE_BUDGET_BYTES: usize = 64 * 1024 * 1024;

/// A cover that could not be decoded. See [`CoverCache::image`].
#[derive(Clone)]
struct Decoded {
    /// `None` when the file could not be read or decoded. Cached so that a
    /// broken cover is attempted **once** rather than on every frame — the
    /// failure is as stable as the success, and the alternative is a decode
    /// attempt per frame for every unreadable file in the library.
    handle: Option<Handle>,
    /// What this entry costs the budget. A failure costs 0 and is evicted
    /// first, which is what a zero-byte entry of the least recent use gets.
    bytes: usize,
}

#[derive(Default)]
struct Inner {
    /// `cover_path → CoverSource`, the PERF-01 memo.
    kinds: HashMap<String, CoverSource>,
    /// `cover_path → decoded pixels`, the PERF-02 cache.
    images: HashMap<String, Decoded>,
    /// Least-recently-used first. Touched on every hit, so the front is the
    /// entry to evict when the budget is exceeded.
    order: VecDeque<String>,
    /// Sum of `Decoded::bytes` over `images`.
    bytes: usize,
    // ---- Counters. Every one of these exists to make a test able to assert a
    // ---- *measurement* rather than a shape; they are not read by the app.
    classify_calls: usize,
    image_calls: usize,
    decode_calls: usize,
}

/// The cover cache: what a game's cover *is*, and what it *looks like*.
///
/// One type rather than two because both are keyed by the same thing (a cover
/// path), both are invalidated by the same events, and both are read from the
/// same place — [`super::widgets`]'s cover builders, which are called once per
/// drawn tile per frame.
///
/// # Interior mutability, and why
///
/// [`crate::view::library::view`] is called from `&self` and takes its inputs
/// as shared references, so a cache that needs `&mut self` on a hit could not
/// be reached from the render path at all. `RefCell` is the same answer iced
/// gives its own raster cache (`iced/tiny_skia/src/raster.rs:123`, a
/// `RefCell<Cache>`) and for the same reason. It is not `Sync`, and does not
/// need to be: every read of it happens on the UI thread, inside `view()`.
///
/// **A borrow is never held across a re-entrant call.** Each method borrows,
/// does its work, and drops the borrow before returning — so the recursion in a
/// widget tree (which calls `cover_plan`, then `framed`, on the same cache) can
/// never see a second `borrow_mut` while one is live.
///
/// # `Debug` is hand-written, not derived
///
/// `crate::State` derives `Debug` and holds one of these
/// (`crates/app/src/state.rs:761`), so the trait is required. Deriving it would
/// print `RefCell<Inner>`'s whole contents — a line per cover path, plus a page
/// of decoded pixel counts — every time anything in the app is formatted with
/// `{:?}`, which is the opposite of useful. This impl prints the counters
/// instead, which are the numbers a reader of a debug dump would want: how
/// many paths are memoised, how many covers are resident, what they cost, and
/// how much filesystem work and decoding has actually happened.
pub struct CoverCache {
    inner: RefCell<Inner>,
    budget: usize,
}

impl std::fmt::Debug for CoverCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.borrow();
        f.debug_struct("CoverCache")
            .field("kinds", &inner.kinds.len())
            .field("images", &inner.images.len())
            .field("bytes", &inner.bytes)
            .field("budget", &self.budget)
            .field("classify_calls", &inner.classify_calls)
            .field("decode_calls", &inner.decode_calls)
            .finish_non_exhaustive()
    }
}

impl Default for CoverCache {
    fn default() -> Self {
        Self::new()
    }
}

impl CoverCache {
    /// A cache with the shipped budget ([`IMAGE_BUDGET_BYTES`]).
    pub fn new() -> Self {
        Self {
            inner: RefCell::new(Inner::default()),
            budget: IMAGE_BUDGET_BYTES,
        }
    }

    /// A cache with an explicit budget, in bytes.
    ///
    /// Exists so a test can drive the eviction path with three small covers
    /// instead of allocating 64 MiB of real ones — the memory bound PERF-02 is
    /// about is not observable from a test that cannot afford to reach it, and
    /// an eviction rule that no test can trigger is a rule nobody has checked.
    /// The app constructs its cache with [`CoverCache::new`].
    pub fn with_budget(bytes: usize) -> Self {
        Self {
            inner: RefCell::new(Inner::default()),
            budget: bytes,
        }
    }

    /// How a game's cover should be drawn, resolved **once per path**.
    ///
    /// The first call for a path performs the filesystem work
    /// ([`CoverSource::classify`]: one `statx`, plus an `openat` and a four-byte
    /// `read` for a file that exists); every later call is a hash lookup. This
    /// is the whole of PERF-01's fix, and the reason `view()` can run every
    /// frame without touching the disk.
    pub fn kind(&self, cover_path: &str) -> CoverSource {
        {
            let inner = self.inner.borrow();
            if let Some(kind) = inner.kinds.get(cover_path) {
                return *kind;
            }
        }

        // Classified *outside* the borrow: `classify` reads the filesystem, and
        // a panic in it must not poison a borrow that the next frame's widgets
        // would then trip over.
        let kind = CoverSource::classify(cover_path);

        let mut inner = self.inner.borrow_mut();
        inner.classify_calls += 1;
        inner.kinds.insert(cover_path.to_string(), kind);
        kind
    }

    /// A game's cover, decoded and bounded, as a handle for the renderer.
    ///
    /// `None` when there is nothing to draw: an empty path, a file that could
    /// not be opened, or bytes that are not an image. The caller draws an empty
    /// box in that case, which is what iced itself does with a handle it cannot
    /// load (`iced/tiny_skia/src/raster.rs:152-160` returns `Err` and `draw`
    /// returns early, `:196`), so a broken cover looks the same before and
    /// after this cache — the difference is that it is *looked at* once.
    ///
    /// The returned handle is stable for as long as the entry is resident:
    /// iced keys its own raster cache by the handle's id
    /// ([`Handle::id`]), and `Handle::from_rgba` mints a **new unique id on
    /// every construction**
    /// (`iced/core/src/image.rs:174-183`) — so rebuilding the handle per frame
    /// would miss that cache every frame and re-upload the pixels. Caching the
    /// handle is what makes the decode-once property hold in the renderer too.
    pub fn image(&self, cover_path: &str) -> Option<Handle> {
        if cover_path.is_empty() {
            return None;
        }

        {
            let mut inner = self.inner.borrow_mut();
            inner.image_calls += 1;
            if inner.images.contains_key(cover_path) {
                touch(&mut inner.order, cover_path);
                return inner
                    .images
                    .get(cover_path)
                    .and_then(|entry| entry.handle.clone());
            }
        }

        // Decoded outside the borrow, for the same reason `kind` classifies
        // outside it: this reads and inflates a file, and holds no lock the
        // view might re-enter.
        let decoded = decode(cover_path);

        let mut inner = self.inner.borrow_mut();
        inner.decode_calls += 1;
        let bytes = decoded
            .as_ref()
            .map_or(0, |(width, height, _)| pixel_bytes(*width, *height));
        let handle =
            decoded.map(|(width, height, pixels)| Handle::from_rgba(width, height, pixels));

        inner.bytes += bytes;
        inner.order.push_back(cover_path.to_string());
        inner.images.insert(
            cover_path.to_string(),
            Decoded {
                handle: handle.clone(),
                bytes,
            },
        );
        evict(&mut inner, self.budget);

        handle
    }

    /// Forget one cover, because the app knows its file was rewritten.
    ///
    /// Called where a cover is replaced in place — the same path, new bytes —
    /// which is the one case a path-keyed cache cannot notice on its own. See
    /// the module docs.
    pub fn forget(&self, cover_path: &str) {
        let mut inner = self.inner.borrow_mut();
        inner.kinds.remove(cover_path);
        if let Some(entry) = inner.images.remove(cover_path) {
            inner.bytes -= entry.bytes;
        }
        inner.order.retain(|path| path != cover_path);
    }

    /// Forget everything, because the library was re-read.
    ///
    /// Wholesale rather than per-game: a library read is the event that can
    /// change *any* entry (a game removed, an id re-pointed at a different
    /// file), and it happens on install, uninstall, form save and startup —
    /// user-visible actions, not a per-frame path — so the coarser invalidation
    /// costs one re-classify per game on the frame after one of them.
    pub fn clear(&self) {
        let mut inner = self.inner.borrow_mut();
        inner.kinds.clear();
        inner.images.clear();
        inner.order.clear();
        inner.bytes = 0;
    }

    /// Bytes of decoded pixels currently held. The quantity [`IMAGE_BUDGET_BYTES`]
    /// bounds.
    pub fn resident_bytes(&self) -> usize {
        self.inner.borrow().bytes
    }

    /// How many cover entries the cache holds.
    ///
    /// Includes entries whose file could not be read or decoded, which are kept
    /// deliberately so a broken cover is attempted once rather than per frame
    /// (see [`Decoded::handle`]). Those cost the budget zero bytes, so this
    /// count and [`CoverCache::resident_bytes`] answer different questions and
    /// are not expected to agree.
    pub fn resident_images(&self) -> usize {
        self.inner.borrow().images.len()
    }

    /// How many classifications have actually read the filesystem.
    ///
    /// The PERF-01 measurement. A frame that draws *n* tiles and touches the
    /// disk zero times leaves this unchanged; the pre-fix code incremented the
    /// equivalent counter twice per tile per frame.
    pub fn classify_calls(&self) -> usize {
        self.inner.borrow().classify_calls
    }

    /// How many times a cover file has been read and decoded.
    pub fn decode_calls(&self) -> usize {
        self.inner.borrow().decode_calls
    }

    /// How many cover images were asked for.
    pub fn image_calls(&self) -> usize {
        self.inner.borrow().image_calls
    }

    /// The byte budget this cache evicts to, as [`CoverCache::new`] set it.
    ///
    /// The eviction tests drive the *rule* through
    /// [`CoverCache::with_budget`], because 64 MiB of real covers is not
    /// something a unit test can allocate. That leaves one line unwatched —
    /// `new` passing [`IMAGE_BUDGET_BYTES`] — and unbinding it is precisely the
    /// pre-fix state PERF-02 describes, so the shipped constructor gets a test
    /// of its own rather than riding on the rule's.
    pub fn budget(&self) -> usize {
        self.budget
    }
}

/// Bytes of one RGBA pixmap.
fn pixel_bytes(width: u32, height: u32) -> usize {
    (width as usize) * (height as usize) * 4
}

/// Move `path` to the most-recently-used end.
fn touch(order: &mut VecDeque<String>, path: &str) {
    if let Some(position) = order.iter().position(|entry| entry == path) {
        order.remove(position);
    }
    order.push_back(path.to_string());
}

/// Drop least-recently-used entries until the budget is met.
///
/// The most recent entry is never evicted, even when it alone exceeds the
/// budget: evicting it would leave the cache holding nothing and decoding that
/// same cover again on the very next frame — a cache miss per frame, which is
/// strictly worse than holding one oversized entry. `CoverCache::with_budget`'s
/// doc says why a test can reach this branch.
fn evict(inner: &mut Inner, budget: usize) {
    while inner.bytes > budget && inner.order.len() > 1 {
        let Some(oldest) = inner.order.pop_front() else {
            break;
        };
        if let Some(entry) = inner.images.remove(&oldest) {
            inner.bytes -= entry.bytes;
        }
    }
}

/// Read, decode and bound one cover file.
///
/// `None` on any failure — an unreadable file, a format no codec claims, a
/// header whose dimensions exceed [`MAX_SOURCE_AXIS`], or zero-sized pixels. Every
/// one of those is external data and none of them is a reason to take the app
/// down; the caller draws an empty box. `image` is already in this build's
/// dependency graph (libcosmic decodes covers through it), so this adds no
/// crate to the lockfile — only a feature set already unified by
/// `libcosmic/animated-image` (`gif`, `webp`, `png`) and libcosmic's own
/// (`ico`, `jpeg`).
fn decode(cover_path: &str) -> Option<(u32, u32, Vec<u8>)> {
    let reader = image::ImageReader::open(cover_path).ok()?;
    let mut reader = reader.with_guessed_format().ok()?;
    // Bounded *before* the decode, not after: an image bomb is a header that
    // claims 60,000 × 60,000, and a cap applied after `decode()` has already
    // allocated it. These are `image`'s own strict limits, and it applies them
    // to the header (`image/src/io/limits.rs:77`, `check_dimensions`) rather
    // than to the buffer, which is the order that matters.
    reader.limits(read_limits());
    let image = reader.decode().ok()?;

    let (width, height) = (image.width(), image.height());
    if width == 0 || height == 0 {
        return None;
    }

    let image = if width > DECODE_MAX || height > DECODE_MAX {
        // `thumbnail` fits the source inside the box and keeps the aspect, which
        // is the reference's `sourceSize` behaviour, and it is a box filter that
        // only ever downsamples.
        image.thumbnail(DECODE_MAX, DECODE_MAX)
    } else {
        image
    };

    let rgba = image.into_rgba8();
    Some((rgba.width(), rgba.height(), rgba.into_raw()))
}

/// The decode limits [`decode`] applies, named so a test can assert the same
/// edge rather than a second copy of the numbers.
///
/// # Why this is a function and not a `const`
///
/// `image::Limits` is `#[non_exhaustive]` (`image/src/io/limits.rs:31`), so a
/// struct expression cannot be written outside that crate — and a `const` that
/// silently dropped a future fourth limit would be the wrong kind of quiet.
/// `Limits::default()` sets `max_alloc` to 512 MiB and leaves both dimensions
/// unlimited (`:49-57`), so the two dimensions are the only ones this has to
/// supply.
///
/// # Why the dimensions and not just `max_alloc`
///
/// `max_alloc` is the limit that is *checked while decoding* and it counts the
/// output buffer, which `image` allocates after the header is read — so without
/// a dimension cap a 60,000² claim is an allocation that fails, not a decode
/// that never starts. [`MAX_SOURCE_AXIS`] is the cap that runs on the header.
fn read_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_AXIS);
    limits.max_image_height = Some(MAX_SOURCE_AXIS);
    // 64 MiB, the same order as [`MAX_SOURCE_AXIS`] squared × 4 = 256 MiB / 4;
    // the dimension cap is what refuses an image bomb, and this one bounds the
    // buffer of an image that passes it.
    limits.max_alloc = Some(64 * 1024 * 1024);
    limits
}

/// The largest source image, on either axis, this cache will even attempt.
///
/// Far above any real cover — the widest thing `COVER_ASSETS` ships is a
/// portrait a few hundred pixels across, and the audit's stress case was
/// 600×900 — and far below the point where a claimed dimension is itself the
/// problem. A cover above it is refused rather than downscaled, because
/// refusing costs one empty box and downscaling costs the allocation this
/// exists to avoid.
pub const MAX_SOURCE_AXIS: u32 = 8_192;

#[cfg(test)]
mod tests {
    use super::*;

    /// The real leading bytes of an ICO, so [`CoverSource::classify`] answers
    /// `Icon` rather than `Photo` for it. Four bytes are all it reads.
    const ICO_PREFIX: &[u8] = &[0x00, 0x00, 0x01, 0x00, 0x01, 0x00];

    /// A fresh, empty directory for one test.
    fn scratch(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gh-cover-cache-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a writable temp directory");
        dir
    }

    /// A real PNG of exactly `width × height`, written to `dir/name`.
    ///
    /// A real encoder rather than a header stub, because the tests below assert
    /// what the *decoder* produced: dimensions and pixel count. A file that
    /// only claims to be a PNG would exercise the failure path instead.
    fn write_png(dir: &std::path::Path, name: &str, width: u32, height: u32) -> String {
        let path = dir.join(name);
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
        image
            .save_with_format(&path, image::ImageFormat::Png)
            .expect("the fixture encodes");
        path.to_string_lossy().into_owned()
    }

    // ---- PERF-01: the classification is a once-per-path cost ----------------

    /// **The finding, as a measurement.** A hundred frames over the same cover
    /// read the filesystem once.
    ///
    /// The pre-fix path called `CoverSource::classify` — which is `is_file()`
    /// plus, for a file that exists, an `open` and a four-byte `read` — twice
    /// per rendered cover per frame, so 100 frames of one tile was 200
    /// classifications and ~200 syscalls. This is the number the audit measured
    /// at 6,624 syscalls over 100 redraws; the assertion is on the counter
    /// rather than on a syscall count because a unit test cannot strace itself,
    /// and `CoverCache::classify_calls` counts exactly the calls that would
    /// strace as a `statx`/`openat`.
    ///
    /// The control is in the test: the *uncached* call is made beside the cached
    /// one and shown to keep working, so a `kind` that stopped classifying
    /// anything (returning `Plate` for a real photo, say) fails here instead of
    /// passing as "no filesystem work".
    #[test]
    fn a_cover_is_classified_once_no_matter_how_many_frames_ask() {
        let dir = scratch("classify-once");
        let path = write_png(&dir, "cover.png", 8, 8);
        let cache = CoverCache::new();

        assert_eq!(cache.classify_calls(), 0);
        for _ in 0..100 {
            assert_eq!(cache.kind(&path), CoverSource::Photo);
        }
        assert_eq!(
            cache.classify_calls(),
            1,
            "100 frames, one stat and one open"
        );

        // The uncached function is still the authority on the answer, and it
        // still reads the file — so "no work" cannot be achieved by answering
        // `Plate` to everything.
        assert_eq!(CoverSource::classify(&path), CoverSource::Photo);
    }

    /// A game with no cover is cached too, and that is the case the audit
    /// measured separately: with the cover files absent the same harness saw
    /// **exactly 333 `statx` per frame**. The empty path and the missing file
    /// are two different keys and two different `Plate` answers.
    #[test]
    fn a_game_with_no_cover_is_classified_once_too() {
        let cache = CoverCache::new();
        for _ in 0..50 {
            assert_eq!(cache.kind(""), CoverSource::Plate);
            assert_eq!(
                cache.kind("/nonexistent/definitely/not/here.jpg"),
                CoverSource::Plate
            );
        }
        assert_eq!(cache.classify_calls(), 2, "one per distinct path");
    }

    /// The cache is keyed by path, so two games sharing a cover file classify
    /// it once, and two games with different files classify twice.
    #[test]
    fn the_key_is_the_path_not_the_game() {
        let dir = scratch("key-is-path");
        let shared = write_png(&dir, "shared.png", 8, 8);
        let icon_path = dir.join("icon.ico");
        std::fs::write(&icon_path, ICO_PREFIX).expect("the fixture writes");
        let icon = icon_path.to_string_lossy().into_owned();
        let cache = CoverCache::new();

        assert_eq!(cache.kind(&shared), CoverSource::Photo);
        assert_eq!(cache.kind(&shared), CoverSource::Photo);
        assert_eq!(cache.kind(&icon), CoverSource::Icon);
        assert_eq!(cache.classify_calls(), 2);
    }

    /// **The staleness the module docs admit, and the fix for it.** A path
    /// whose *bytes* change keeps its old answer until the app says so, because
    /// nothing else can know without a `stat` — and a `stat` per path is the
    /// defect.
    ///
    /// This is the test that pins the admitted limit rather than leaving it a
    /// sentence in a doc comment: it fails if `kind` ever starts re-reading the
    /// file per call (the `still Photo` line), and it fails if `forget` stops
    /// dropping the entry.
    #[test]
    fn a_rewritten_cover_keeps_its_answer_until_the_app_forgets_it() {
        let dir = scratch("rewritten");
        let path = write_png(&dir, "cover.png", 8, 8);
        let cache = CoverCache::new();

        assert_eq!(cache.kind(&path), CoverSource::Photo);
        std::fs::write(&path, ICO_PREFIX).expect("the fixture is rewritten");
        assert_eq!(
            cache.kind(&path),
            CoverSource::Photo,
            "the entry is keyed by path and holds the answer for the file that was there"
        );
        assert_eq!(cache.classify_calls(), 1);

        cache.forget(&path);
        assert_eq!(cache.kind(&path), CoverSource::Icon);
        assert_eq!(cache.classify_calls(), 2);
    }

    /// [`CoverCache::clear`] is the library-re-read invalidation, and it drops
    /// both halves: the kinds *and* the decoded pixels, because a library that
    /// was re-read may point at different files.
    #[test]
    fn clearing_the_library_drops_every_entry() {
        let dir = scratch("clear");
        let path = write_png(&dir, "cover.png", 8, 8);
        let cache = CoverCache::new();
        assert_eq!(cache.kind(&path), CoverSource::Photo);
        assert!(cache.image(&path).is_some());
        assert!(cache.resident_bytes() > 0);

        cache.clear();

        assert_eq!(cache.resident_bytes(), 0);
        assert_eq!(cache.resident_images(), 0);
        assert_eq!(cache.classify_calls(), 1);
        assert_eq!(cache.kind(&path), CoverSource::Photo);
        assert_eq!(cache.classify_calls(), 2, "cleared, so it classified again");
    }

    // ---- PERF-02: the pixels are bounded ------------------------------------

    /// **The finding, as a measurement.** A 600×900 cover — the audit's own
    /// size, the one that took a 500-game library to 753 MB — is decoded to
    /// 512 in its longest axis, and what the cache reports it holds is the
    /// number this test computes for itself from the decoded dimensions.
    ///
    /// 600 × 900 × 4 = 2,160,000 bytes at source resolution; 341 × 512 × 4 =
    /// 698,368 as decoded. The aspect is preserved and nothing is upscaled.
    #[test]
    fn a_cover_larger_than_the_cap_is_decoded_no_larger_than_the_cap() {
        let dir = scratch("bounded");
        let path = write_png(&dir, "big.png", 600, 900);
        let cache = CoverCache::new();

        let handle = cache.image(&path).expect("a decodable PNG");

        let Handle::Rgba { width, height, .. } = handle else {
            panic!("this cache hands the renderer decoded pixels, not a path");
        };
        assert_eq!(
            (width, height),
            (341, 512),
            "fit inside the cap, aspect kept"
        );
        assert!(
            width <= DECODE_MAX && height <= DECODE_MAX,
            "the cap is per axis, as the QML's sourceSize is"
        );
        assert_eq!(cache.resident_bytes(), 341 * 512 * 4);
        assert!(
            cache.resident_bytes() < 600 * 900 * 4,
            "bounded, not merely recorded: {} against the source's {}",
            cache.resident_bytes(),
            600 * 900 * 4
        );
    }

    /// A cover already smaller than the cap is not resampled and not upscaled —
    /// the reference's `sourceSize` only ever shrinks, and an upscaled decode is
    /// both wrong and more expensive.
    #[test]
    fn a_cover_under_the_cap_is_decoded_at_its_own_size() {
        let dir = scratch("small");
        let path = write_png(&dir, "small.png", 100, 150);
        let cache = CoverCache::new();

        let Some(Handle::Rgba { width, height, .. }) = cache.image(&path) else {
            panic!("a decodable PNG");
        };
        assert_eq!((width, height), (100, 150));
        assert_eq!(cache.resident_bytes(), 100 * 150 * 4);
    }

    /// One decode per cover, and the *same* handle handed back every time.
    ///
    /// The second half is the load-bearing one: iced keys its own raster cache
    /// by the handle's id, and `Handle::from_rgba` mints a new unique id on
    /// every construction, so a cache that re-minted the handle per call would
    /// make iced re-copy the pixels on every frame while this test still saw
    /// one decode. The ids are compared, not the pointers.
    #[test]
    fn a_cover_is_decoded_once_and_the_handle_is_the_same_one() {
        let dir = scratch("decode-once");
        let path = write_png(&dir, "cover.png", 40, 60);
        let cache = CoverCache::new();

        let first = cache.image(&path).expect("a decodable PNG");
        for _ in 0..20 {
            let again = cache.image(&path).expect("still resident");
            assert_eq!(again.id(), first.id());
        }
        assert_eq!(cache.decode_calls(), 1);
        assert_eq!(cache.image_calls(), 21);
    }

    /// **The budget.** With room for two covers, a third evicts the least
    /// recently used one — and the one that was *touched* between the two
    /// decodes is the one that survives, which is what makes this a least-
    /// recently-*used* rule rather than a queue.
    #[test]
    fn the_cache_evicts_the_least_recently_used_cover_at_the_budget() {
        let dir = scratch("budget");
        let (a, b, c) = (
            write_png(&dir, "a.png", 40, 40),
            write_png(&dir, "b.png", 40, 40),
            write_png(&dir, "c.png", 40, 40),
        );
        // Two 40×40 covers fit; the third does not.
        let cache = CoverCache::with_budget(40 * 40 * 4 * 2);

        assert!(cache.image(&a).is_some());
        assert!(cache.image(&b).is_some());
        assert_eq!(cache.resident_images(), 2);
        assert!(cache.resident_bytes() <= 40 * 40 * 4 * 2);

        // `a` is used again, so `b` is now the least recently used.
        assert!(cache.image(&a).is_some());
        assert!(cache.image(&c).is_some());

        assert_eq!(cache.resident_images(), 2, "the budget is a ceiling");
        assert!(cache.resident_bytes() <= 40 * 40 * 4 * 2);
        // `a` stayed, `b` went, `c` arrived: asserting *which* survived is what
        // separates LRU from "drop whichever the hash map iterates first".
        assert_eq!(cache.decode_calls(), 3, "evicted entries are re-decoded");
        assert!(cache.image(&b).is_some(), "and a re-request works");
        assert_eq!(cache.decode_calls(), 4);
    }

    /// The eviction rule above is driven through [`CoverCache::with_budget`],
    /// so it says nothing about the budget the *app* gets. This is that half:
    /// the constructor the app calls is bounded, and bounded by the documented
    /// number.
    ///
    /// It is the seam PERF-02 turns on. Pre-fix, residency was bounded by
    /// nothing in this app — iced's own trim keeps what the last frame drew,
    /// which for a library page is every game it drew — so a `new` that passed
    /// `usize::MAX` here would restore the finding exactly while every other
    /// test in this module still passed. Measured end-to-end against the real
    /// binary at 333 covers, that difference is 774,840 kB of RSS against
    /// 89,780 kB.
    #[test]
    fn the_shipped_cache_is_bounded_by_the_documented_budget() {
        assert_eq!(CoverCache::new().budget(), IMAGE_BUDGET_BYTES);
        assert_eq!(CoverCache::default().budget(), IMAGE_BUDGET_BYTES);
        // And the documented number is a bound a library page can live inside.
        // The assertion is the *claim the doc makes* — "the drawn set fits" — and
        // not merely "the budget is non-zero": a `IMAGE_BUDGET_BYTES` of 1 MiB
        // would satisfy `>= one cover` while being the pre-fix state for PERF-02
        // (the entry just decoded would be evicted to make room for itself, since
        // `evict` never drops the most recent entry and so the cache would sit
        // permanently over budget). The drawn set is a measured 45 cards, so the
        // budget must hold one whole window at the cap — 45 MiB — before it can
        // hold anything else.
        //
        // The first version of this line asserted `16 *` that, which is 720 MiB
        // and fails against the shipped 64 MiB. It was written from the intent
        // ("many windows") rather than from the arithmetic, and the test caught
        // it — which is the only reason to state the multiple explicitly here
        // instead of in a comment.
        const DRAWN_CARDS_MEASURED: usize = 45;
        let one_window_at_cap =
            DRAWN_CARDS_MEASURED * DECODE_MAX as usize * DECODE_MAX as usize * 4;
        assert!(
            IMAGE_BUDGET_BYTES >= one_window_at_cap,
            "the budget must hold a whole window at the cap ({one_window_at_cap} bytes), \
             not one cover"
        );
    }

    /// A cover that cannot be decoded is attempted **once**, not once per
    /// frame, and it is reported as absent so the caller draws an empty box
    /// rather than panicking.
    ///
    /// Both halves matter on the render path: an unreadable file in a library
    /// is external state (a deleted cover, a truncated download), and neither a
    /// panic nor a decode attempt per frame is an acceptable answer to it.
    #[test]
    fn an_undecodable_cover_is_attempted_once_and_reported_absent() {
        let dir = scratch("broken");
        let path = dir.join("broken.jpg");
        std::fs::write(&path, b"not an image at all").expect("the fixture writes");
        let path = path.to_string_lossy().into_owned();
        let cache = CoverCache::new();

        for _ in 0..10 {
            assert!(cache.image(&path).is_none());
        }
        assert_eq!(cache.decode_calls(), 1, "one attempt, ten frames");
        assert_eq!(cache.resident_bytes(), 0, "a failure costs no budget");
        assert_eq!(
            cache.resident_images(),
            1,
            "but it IS a resident entry — see `resident_images`: the count includes \
             the failures, which is why it and `resident_bytes` answer different \
             questions"
        );
        assert!(cache.image("").is_none(), "an empty path is never a file");
        assert_eq!(cache.decode_calls(), 1, "and is not an attempt either");
    }

    /// A file the size cap rejects is refused rather than allocated.
    ///
    /// The header is produced by a real encoder at 9,000 × 9,000 — above
    /// [`MAX_SOURCE_AXIS`]'s 8,192 per axis, and 324 MB if it were decoded — so this
    /// pins the limit that runs *before* the allocation rather than a limit
    /// applied to something already in memory. It is one of the two external-
    /// data cases the module handles: the other is the broken file above.
    #[test]
    fn a_cover_above_the_read_limit_is_refused_without_being_decoded() {
        let dir = scratch("too-big");
        let path = write_png(&dir, "huge.png", 9_000, 9_000);
        let cache = CoverCache::new();

        assert!(cache.image(&path).is_none());
        assert_eq!(cache.resident_bytes(), 0);
        assert_eq!(cache.decode_calls(), 1);

        // And the limit is the number the module names, not a number that
        // happens to reject this fixture: a cover inside it still decodes.
        let inside = write_png(&dir, "inside.png", 8_192, 1);
        assert!(cache.image(&inside).is_some());
    }

    /// A cover that is one pixel over the cap on one axis only is still
    /// downscaled, and one exactly at the cap is left alone. The boundary is
    /// where an off-by-one would show, and the two sides of it must not both
    /// take the same branch.
    #[test]
    fn the_cap_is_inclusive_at_exactly_the_cap() {
        let dir = scratch("boundary");
        let at_cap = write_png(&dir, "at.png", 512, 512);
        let over_cap = write_png(&dir, "over.png", 513, 512);
        let cache = CoverCache::new();

        let Some(Handle::Rgba { width, height, .. }) = cache.image(&at_cap) else {
            panic!("a decodable PNG");
        };
        assert_eq!((width, height), (512, 512), "at the cap is not over it");

        let Some(Handle::Rgba { width, height, .. }) = cache.image(&over_cap) else {
            panic!("a decodable PNG");
        };
        assert_eq!(width, 512, "one pixel over is downscaled to the cap");
        assert_eq!(height, 511, "and the aspect is kept");
    }
}
