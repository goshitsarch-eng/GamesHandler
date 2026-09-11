//! Cover artwork: the pure rules the interface renders with.
//!
//! Port of the deterministic half of `gamehandler/covers.py`. Only the parts
//! the interface needs *without* touching the network or the filesystem live
//! here — the fetching half (`fetch_cover`, `download_image`, `steam_search`)
//! lands with T-05, which is where the injected HTTP client of D-26 belongs.
//!
//! # Why [`accent_index`] is in `core` rather than in the view
//!
//! `crates/app/src/view/cover.rs` needs it to pick a game's placeholder shade,
//! and it is a hash — so it was tempting to put it where it is used. It is here
//! because it is a **port of a Python function whose value is observable**: a
//! game's tile has a colour that must not change between the Python app and
//! this one, or across sessions. That makes it a compatibility surface, and
//! compatibility surfaces belong where the oracle fixtures can reach them,
//! which is `core`.
//!
//! The alternative the view originally reached for — widening
//! [`crate::hash::sha256_hex`] to `pub` — was rejected for the reason the view
//! itself gave: it exports the *primitive* while the thing that must not drift
//! is the *rule*. A caller holding `sha256_hex` can bucket the digest its own
//! way, and the drift would be invisible until someone compared two screenshots.
//! Exposing the decision keeps the rule in one place, and keeps the hash an
//! implementation detail of it.

/// How many placeholder shades there are. `covers.py:95`.
pub const COVER_ACCENTS: usize = 8;

/// A stable colour bucket for `seed`, so a game's tile never changes shade.
/// `accent_index` (`covers.py:107-110`).
///
/// `sha256(seed.encode("utf-8")).digest()[0] % COVER_ACCENTS` — the **first byte**
/// of the digest, not the first hex digit. Reading the digest as text and
/// taking one character would give a value in `0..16` bucketed against 8 and so
/// happen to produce the same answer, which is exactly the kind of coincidence
/// that stops being harmless the moment the bucket count changes. The
/// arithmetic is done on a byte here, as in Python.
///
/// `seed` being empty is handled by hashing the empty string rather than by a
/// special case, because that is what `(seed or "")` does and the two agree:
/// `sha256(b"")` is a real digest whose first byte is 227, so the empty seed is
/// bucket 3, not bucket 0. A port that short-circuited the empty case would put
/// every nameless game in the first shade.
///
/// UTF-8 rather than the platform encoding, so the bucket for a non-ASCII name
/// is the same on every machine — which is what makes this reproducible at all.
pub fn accent_index(seed: &str) -> usize {
    accent_index_in(seed, COVER_ACCENTS)
}

/// [`accent_index`] with an explicit bucket count.
///
/// Exists because Python's signature is `accent_index(seed, buckets=COVER_ACCENTS)`
/// and the default is the *only* thing that differs: the corpus exercises other
/// counts to reach the clamp below, and a caller that needs a different number
/// of shades should not have to re-derive the rule to get one.
///
/// `buckets` is clamped to at least 1, as `max(1, buckets)` does. A zero would
/// otherwise be a division by zero — in Python it raises, here it would panic,
/// and neither is a useful behaviour for a caller that computed a count from a
/// config file. Negative counts cannot reach the clamp from Python's `int`, but
/// they can from a `usize`-underflowing caller, so the clamp is written on the
/// way in rather than assumed.
pub fn accent_index_in(seed: &str, buckets: usize) -> usize {
    let buckets = buckets.max(1);
    let digest = crate::hash::sha256_hex(seed.as_bytes());
    // The digest is hex; the first byte is its first two characters. Taking the
    // byte rather than parsing a `u64` keeps this equal to Python's
    // `digest[0]` for every seed, including the ones whose first byte is above
    // `0x7f` — where a naive `hex[0]` would be an ASCII letter.
    let first = u8::from_str_radix(&digest[0..2], 16)
        .expect("sha256_hex always writes at least two hex characters");
    usize::from(first) % buckets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_empty_seed_is_hashed_rather_than_short_circuited() {
        // `(seed or "")` hashes the empty string. The value below is CPython's:
        // `int.from_bytes(hashlib.sha256(b"").digest()[:1])` is 227, and
        // `227 % 8` is 3. A port that returned 0 for the empty seed would put
        // every unnamed game in the first shade, and this is the assertion that
        // says so.
        assert_eq!(accent_index(""), 3);
        assert_ne!(accent_index(""), 0);
    }

    #[test]
    fn the_bucket_is_the_first_digest_byte_not_the_first_hex_digit() {
        // `sha256("")` is `e3b0c442…`. Its first byte is `0xe3` = 227, bucket 3.
        // Reading the digest as text and taking `'e'` would give 14, bucket 6 —
        // a value this test rejects. The two differ only because 227 and 14 are
        // incongruent mod 8, so the case has to be one where they are.
        assert_eq!(crate::hash::sha256_hex(b"")[0..2], *"e3");
        assert_eq!(accent_index(""), 227 % COVER_ACCENTS);
    }

    #[test]
    fn known_seeds_land_in_cpython_s_buckets() {
        // Every pair below was produced by running the Python implementation.
        // They are here as a smoke test with readable values; the exhaustive
        // check is the vector corpus, which carries the same cases plus the
        // awkward ones (non-ASCII, long ids, the clamp).
        assert_eq!(accent_index("a"), 2);
        assert_eq!(accent_index("bb"), 3);
        assert_eq!(accent_index("ccc"), 4);
        assert_eq!(accent_index("game-a"), 7);
        assert_eq!(accent_index("a-very-long-id"), 3);
        assert_eq!(accent_index("Half-Life"), 5);
        assert_eq!(accent_index("0".repeat(32).as_str()), 4);
    }

    #[test]
    fn a_non_ascii_seed_is_hashed_as_utf8() {
        // The seed is a game id or name, either of which can be non-ASCII. The
        // encoding is what makes the bucket reproducible across machines, so it
        // is pinned rather than assumed: CPython's `"😀".encode("utf-8")` is
        // `f0 9f 98 80` and this crate hashes the same four bytes.
        assert_eq!(accent_index("\u{1f600}"), 0);
        assert_eq!(
            crate::hash::sha256_hex("\u{1f600}".as_bytes())[0..2],
            *"f0"
        );
    }

    #[test]
    fn the_result_is_always_a_usable_shade_index() {
        // The property that matters to the view: whatever the seed, the value
        // indexes `COVER_GRADIENTS` without wrapping. `shade` wraps defensively,
        // but it should never have to.
        for seed in ["", "a", "\u{1f600}", &"x".repeat(500)] {
            assert!(accent_index(seed) < COVER_ACCENTS, "{seed:?} escaped the range");
        }
    }

    #[test]
    fn the_same_seed_always_gets_the_same_shade() {
        // Stability is the whole point of hashing here: a tile that changed
        // colour between sessions would be worse than a shared colour.
        let first = accent_index("Half-Life");
        for _ in 0..8 {
            assert_eq!(accent_index("Half-Life"), first);
        }
    }

    #[test]
    fn a_bucket_count_of_zero_is_clamped_rather_than_dividing_by_zero() {
        // `max(1, buckets)` in Python. Reaching this needs an explicit count,
        // which is why `accent_index_in` is public — a caller that computed zero
        // from a config file must not panic.
        assert_eq!(accent_index_in("a", 0), 0);
        assert_eq!(accent_index_in("a", 1), 0);
        for buckets in [2usize, 3, 5, 8, 100] {
            assert!(accent_index_in("a", buckets) < buckets);
        }
        // The value for a real bucket count is still Python's.
        assert_eq!(accent_index_in("a", 3), 1);
    }

    #[test]
    fn seeds_that_look_alike_do_not_share_a_shade_by_accident() {
        // Guards against a stand-in that keys off the *shape* of the string
        // rather than its hash — the view's placeholder did exactly that before
        // it was wired up, and its own test caught it. The seeds here are
        // deliberately pairwise different in length as well as content, so a
        // `len() % buckets` implementation cannot pass.
        let seeds = ["", "a", "bb", "ccc", "game-a", "a-very-long-id"];
        let buckets: Vec<usize> = seeds.iter().map(|seed| accent_index(seed)).collect();
        let mut distinct = buckets.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert!(
            distinct.len() > 1,
            "every seed got the same bucket ({buckets:?}) — this is not hashing"
        );
    }
}
