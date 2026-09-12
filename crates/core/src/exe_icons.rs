//! Read the icon a Windows executable carries in its PE resource section.
//!
//! Port of `gamehandler/exe_icons.py` (T-05, first of the two remaining rows:
//! `netpaths` landed at `aa54275`, this module and `covers`' art half are what
//! `docs/migration/PLAN.md:349` still asks for). Store launchers and most
//! Windows apps are not Steam store products, so a Steam cover lookup finds
//! nothing for them — or, worse, something else with a similar name. The
//! executable the vendor's own installer just wrote is a source that is always
//! present, always the right artwork, needs no network, and redistributes
//! nothing: the icon comes out of the user's own install.
//!
//! This is P-58's vendor-icon dependency: T-38 cannot finish the
//! finished-install-to-library-entry step without it, and `save_exe_icon`
//! ([`crate::covers::save_exe_icon`]) is the only writer of `.ico` covers.
//!
//! # Faithful where it matters, adapted where Rust demands it
//!
//! The parser itself ([`icon_bytes`]) mirrors the reference line for line,
//! including the walk order: icon images accumulate in resource-directory
//! order (duplicates replaced in place, as the reference's dict does), while
//! icon *groups* are tried lowest-numbered first, because that is the group
//! Windows shows as the application icon (`exe_icons.py:227`).
//!
//! Two things differ, both deliberately:
//!
//! - The reference memory-maps the executable so a cover lookup never buffers
//!   a whole game (`exe_icons.py:248-252`). `std` has no mmap and this crate
//!   takes no dependency for one, so [`extract_icon`] reads in two phases: a
//!   small header probe, then a bounded read through the resource section's
//!   raw bytes, falling back to a full read only when the bounded view yields
//!   nothing. Same observable contract (`None` for anything unreadable), same
//!   worst case ([`MAX_EXECUTABLE_BYTES`], checked before any read).
//! - Every read is bounds-checked against the bytes actually in hand. The
//!   reference relies on `mmap` raising `ValueError` past the end of the
//!   file — which [`extract_icon`] catches and turns into `None` — while a
//!   `bytes` input would silently clamp. Clamping cannot produce a valid icon
//!   (the group/image structure checks reject short reads), so returning
//!   `None` at the first out-of-range read matches the production (`mmap`)
//!   path exactly.
//!
//! A resource tree is data we did not write, and a truncated or hostile one
//! must cost a moment rather than a gigabyte. Every walk is bounded, exactly
//! as in the reference.

use std::collections::BTreeMap;
use std::path::Path;

/// Resource type id for a single icon image (`exe_icons.py:18`).
pub const RT_ICON: u32 = 3;

/// Resource type id for an icon group (`exe_icons.py:19`).
pub const RT_GROUP_ICON: u32 = 14;

/// Largest executable [`extract_icon`] will look at (`exe_icons.py:23`).
///
/// Checked with `stat` before any byte is read, so a hostile path costs one
/// syscall rather than a half-gigabyte buffer.
pub const MAX_EXECUTABLE_BYTES: u64 = 512 * 1024 * 1024;

/// Most section headers the parser will walk (`exe_icons.py:24`).
const MAX_SECTIONS: usize = 96;

/// Most entries read from one resource directory (`exe_icons.py:25`).
const MAX_DIRECTORY_ENTRIES: u32 = 4096;

/// Most images assembled into one `.ico` (`exe_icons.py:26`).
const MAX_ICON_IMAGES: usize = 64;

/// Most icon payload bytes held at once, in total (`exe_icons.py:27`).
const MAX_ICON_BYTES: usize = 8 * 1024 * 1024;

const PE32_MAGIC: u16 = 0x10B;
const PE32PLUS_MAGIC: u16 = 0x20B;
const RESOURCE_DIRECTORY_INDEX: usize = 2;
const SUBDIRECTORY_FLAG: u32 = 0x80000000;
const NAME_FLAG: u32 = 0x80000000;

/// How much of a large executable the header probe reads.
///
/// PE headers and the section table live at the front of the file (`lfanew`
/// is normally below 4 KiB), so 64 KiB covers every real header with room to
/// spare. Anything past it — including a freak `lfanew` — is what the full
/// fallback read in [`extract_icon`] is for.
const PROBE_BYTES: usize = 64 * 1024;

/// One section header: where it maps and where its bytes are on disk.
///
/// `span` is the larger of the virtual and raw sizes, exactly as the
/// reference computes it: a section's mapped span can exceed its on-disk
/// bytes (BSS-style padding) and vice versa, containment uses the larger of
/// the two, and reads are bounded by what is actually on disk
/// (`exe_icons.py:77-80`).
struct Section {
    virtual_address: u32,
    span: u32,
    raw_pointer: u32,
    raw_size: u32,
}

/// One resource directory entry: `(id, target, is_directory, is_named)`.
struct DirectoryEntry {
    id: u32,
    target: u32,
    is_directory: bool,
    is_named: bool,
}

/// Return the primary icon of an in-memory PE image as `.ico` bytes.
///
/// Port of `icon_bytes` (`exe_icons.py:182-235`). `None` means "no usable
/// icon" — not a PE, truncated, no resources, no groups, no images, or a
/// group that assembles to nothing — never an error, because a cover lookup
/// that cannot read an icon just has no icon.
pub fn icon_bytes(data: &[u8]) -> Option<Vec<u8>> {
    let (sections, resource_rva, _resource_size) = sections_and_resource_root(data)?;
    let base = file_offset(&sections, data.len(), resource_rva, 16)?;

    // `{id: target}` over type directories that are subdirectories and
    // unnamed. A second entry with the same id overwrites the first, as the
    // reference's dict comprehension does.
    let mut icon_type = None;
    let mut group_type = None;
    for entry in directory_entries(data, base, 0) {
        if entry.is_directory && !entry.is_named {
            if entry.id == RT_ICON {
                icon_type = Some(entry.target);
            } else if entry.id == RT_GROUP_ICON {
                group_type = Some(entry.target);
            }
        }
    }
    let (icon_type, group_type) = (icon_type?, group_type?);

    let groups = collect_group_resources(data, base, group_type);
    let icons = collect_resources(data, base, icon_type);
    if groups.is_empty() || icons.is_empty() {
        return None;
    }

    let read = |entry: (u32, u32)| -> Option<Vec<u8>> {
        let (rva, size) = entry;
        if size == 0 || usize::try_from(size).ok()? > MAX_ICON_BYTES {
            return None;
        }
        let size = size as usize;
        let offset = file_offset(&sections, data.len(), rva, size)?;
        data.get(offset..offset.checked_add(size)?)
            .map(<[u8]>::to_vec)
    };

    let mut images: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut total = 0usize;
    for (identifier, entry) in &icons {
        let Some(payload) = read(*entry) else {
            continue;
        };
        total += payload.len();
        if total > MAX_ICON_BYTES {
            break;
        }
        images.push((*identifier, payload));
    }
    if images.is_empty() {
        return None;
    }

    // Windows shows the lowest-numbered icon group as the application icon,
    // and a map's values iterate in key order, so this is `sorted(groups)`.
    for entry in groups.values() {
        let Some(group) = read(*entry) else {
            continue;
        };
        if let Some(ico) = build_ico(&group, &images) {
            return Some(ico);
        }
    }
    None
}

/// Return *exe_path*'s embedded icon as `.ico` bytes, or `None`.
///
/// Port of `extract_icon` (`exe_icons.py:238-254`): a missing file, a
/// directory, anything under 64 bytes or over [`MAX_EXECUTABLE_BYTES`], and
/// anything the parser rejects all yield `None` rather than an error.
///
/// Files up to [`PROBE_BYTES`] are read whole — one syscall, one parse. Larger
/// files take the bounded path: parse the headers from a probe, read through
/// the end of the resource section's raw bytes, and only fall back to a full
/// read when the bounded view yields nothing. The fallback matters: headers
/// past the probe, or resource data pointing outside the resource section,
/// are legal, and a bounded-only read would miss icons the reference finds.
pub fn extract_icon(exe_path: &Path) -> Option<Vec<u8>> {
    let len = std::fs::metadata(exe_path).ok()?.len();
    if !(64..=MAX_EXECUTABLE_BYTES).contains(&len) {
        return None;
    }
    let len = usize::try_from(len).ok()?;
    if len <= PROBE_BYTES {
        return icon_bytes(&read_prefix(exe_path, len)?);
    }
    let probe = read_prefix(exe_path, PROBE_BYTES)?;
    if let Some(ico) = bounded_attempt(exe_path, len, &probe) {
        return Some(ico);
    }
    icon_bytes(&read_prefix(exe_path, len)?)
}

/// Try the resource section's raw bytes before paying for a full read.
///
/// Returns `None` when the bounded view is *inconclusive* — headers that do
/// not parse inside the probe, or a parse that finds no icon — in which case
/// the caller falls back to the whole file. A bounded view that parses to an
/// icon is final: the full file cannot hold a *better* icon for the same
/// headers, only the same bytes at the same offsets.
fn bounded_attempt(exe_path: &Path, len: usize, probe: &[u8]) -> Option<Vec<u8>> {
    let end = bounded_end(probe)?.min(len);
    if end <= probe.len() {
        return icon_bytes(probe);
    }
    icon_bytes(&read_prefix(exe_path, end)?)
}

/// End offset of a bounded read: through the resource section's raw bytes.
///
/// `None` when the probe does not hold parseable headers — the signal to skip
/// the bounded view and read the whole file.
fn bounded_end(probe: &[u8]) -> Option<usize> {
    let (sections, resource_rva, _size) = sections_and_resource_root(probe)?;
    let rva = u64::from(resource_rva);
    sections.iter().find_map(|section| {
        let start = u64::from(section.virtual_address);
        if rva >= start && rva < start + u64::from(section.span) {
            u64::from(section.raw_pointer)
                .checked_add(u64::from(section.raw_size))
                .and_then(|end| usize::try_from(end).ok())
        } else {
            None
        }
    })
}

/// Read exactly the first `len` bytes of a file.
///
/// `None` on anything the reference's `open`/`mmap` pair would refuse —
/// missing, unreadable, a directory, or shrunk between the `stat` and the
/// read — so every failure folds into "no icon", as it does there.
fn read_prefix(exe_path: &Path, len: usize) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let mut file = std::fs::File::open(exe_path).ok()?;
    let mut buffer = vec![0u8; len];
    file.read_exact(&mut buffer).ok()?;
    Some(buffer)
}

/// Parse the PE headers into `(sections, resource_rva, resource_size)`.
///
/// Port of `_sections_and_resource_root` (`exe_icons.py:36-83`). Every offset
/// is checked before it is read: where the reference guards with explicit
/// length comparisons (and, past them, relies on `struct.error`/`ValueError`
/// from the caller), this returns `None` at the first out-of-range read.
fn sections_and_resource_root(data: &[u8]) -> Option<(Vec<Section>, u32, u32)> {
    if data.len() < 64 || data.get(0..2)? != b"MZ" {
        return None;
    }
    let lfanew = u32_at(data, 0x3C)?;
    if lfanew == 0 {
        return None;
    }
    let lfanew = lfanew as usize;
    if lfanew.checked_add(24)? > data.len() {
        return None;
    }
    if data.get(lfanew..lfanew.checked_add(4)?)? != b"PE\x00\x00" {
        return None;
    }

    let section_count = u16_at(data, lfanew.checked_add(6)?)?;
    let optional_size = u16_at(data, lfanew.checked_add(20)?)?;
    let optional = lfanew.checked_add(24)?;
    if optional_size < 2 || optional.checked_add(optional_size as usize)? > data.len() {
        return None;
    }
    let magic = u16_at(data, optional)?;
    let (count_offset, directory_offset) = match magic {
        PE32_MAGIC => (92usize, 96usize),
        PE32PLUS_MAGIC => (108usize, 112usize),
        _ => return None,
    };
    let directories_end = optional
        .checked_add(directory_offset)?
        .checked_add(8 * (RESOURCE_DIRECTORY_INDEX + 1))?;
    if directories_end > data.len() {
        return None;
    }
    let directory_count = u32_at(data, optional.checked_add(count_offset)?)?;
    if directory_count <= RESOURCE_DIRECTORY_INDEX as u32 {
        return None;
    }
    let entry = optional
        .checked_add(directory_offset)?
        .checked_add(8 * RESOURCE_DIRECTORY_INDEX)?;
    let resource_rva = u32_at(data, entry)?;
    let resource_size = u32_at(data, entry.checked_add(4)?)?;
    if resource_rva == 0 || resource_size == 0 {
        return None;
    }

    let table = optional.checked_add(optional_size as usize)?;
    let mut sections = Vec::new();
    for index in 0..usize::from(section_count).min(MAX_SECTIONS) {
        let position = table.checked_add(index.checked_mul(40)?)?;
        if position.checked_add(40)? > data.len() {
            break;
        }
        let virtual_size = u32_at(data, position.checked_add(8)?)?;
        let virtual_address = u32_at(data, position.checked_add(12)?)?;
        let raw_size = u32_at(data, position.checked_add(16)?)?;
        let raw_pointer = u32_at(data, position.checked_add(20)?)?;
        sections.push(Section {
            virtual_address,
            span: virtual_size.max(raw_size),
            raw_pointer,
            raw_size,
        });
    }
    if sections.is_empty() {
        return None;
    }
    Some((sections, resource_rva, resource_size))
}

/// Translate a relative virtual address into a readable file offset.
///
/// Port of `_file_offset` (`exe_icons.py:86-94`), plus the end-of-data check
/// the reference gets from `mmap`: a section whose raw range runs past the
/// bytes in hand yields `None` here, as the `ValueError` the reference's
/// caller catches does there. Arithmetic is `u64` throughout so a hostile
/// header cannot wrap a containment check.
fn file_offset(sections: &[Section], data_len: usize, rva: u32, length: usize) -> Option<usize> {
    let rva = u64::from(rva);
    let length = length as u64;
    for section in sections {
        let start = u64::from(section.virtual_address);
        if rva >= start && rva < start + u64::from(section.span) {
            let delta = rva - start;
            if delta + length > u64::from(section.raw_size) {
                return None;
            }
            let offset = u64::from(section.raw_pointer) + delta;
            if offset + length > data_len as u64 {
                return None;
            }
            return usize::try_from(offset).ok();
        }
    }
    None
}

/// `(id, target, is_directory, is_named)` for one resource directory.
///
/// Port of `_directory_entries` (`exe_icons.py:97-118`).
fn directory_entries(data: &[u8], base: usize, offset: u32) -> Vec<DirectoryEntry> {
    let Some(start) = base.checked_add(offset as usize) else {
        return Vec::new();
    };
    if start.checked_add(16).is_none_or(|end| end > data.len()) {
        return Vec::new();
    }
    let (Some(named), Some(numbered)) = (u16_at(data, start + 12), u16_at(data, start + 14)) else {
        return Vec::new();
    };
    let total = u32::from(named) + u32::from(numbered);
    let mut entries = Vec::new();
    for index in 0..total.min(MAX_DIRECTORY_ENTRIES) {
        let Some(position) = start
            .checked_add(16)
            .and_then(|start| start.checked_add(index as usize * 8))
        else {
            break;
        };
        if position.checked_add(8).is_none_or(|end| end > data.len()) {
            break;
        }
        let (Some(name), Some(target)) = (u32_at(data, position), u32_at(data, position + 4))
        else {
            break;
        };
        entries.push(DirectoryEntry {
            id: name & !NAME_FLAG,
            target: target & !SUBDIRECTORY_FLAG,
            is_directory: target & SUBDIRECTORY_FLAG != 0,
            is_named: name & NAME_FLAG != 0,
        });
    }
    entries
}

/// `{resource id: (data rva, size)}` under one resource *type* directory.
///
/// Port of `_collect_resources` (`exe_icons.py:121-141`). The pairs keep
/// directory-walk order in a vector rather than a map because the caller
/// spends the icon budget in that order; a repeated id replaces its earlier
/// pair in place, which is what assigning the reference's dict twice does.
/// Only the first language of an icon is collected — the localisations are
/// the same artwork with different locale metadata.
fn collect_resources(data: &[u8], base: usize, offset: u32) -> Vec<(u32, (u32, u32))> {
    let mut found: Vec<(u32, (u32, u32))> = Vec::new();
    for entry in directory_entries(data, base, offset) {
        if entry.is_named || !entry.is_directory {
            continue;
        }
        for leaf in directory_entries(data, base, entry.target) {
            if leaf.is_directory {
                continue;
            }
            let Some(position) = base.checked_add(leaf.target as usize) else {
                break;
            };
            if position.checked_add(16).is_none_or(|end| end > data.len()) {
                break;
            }
            let (Some(data_rva), Some(data_size)) =
                (u32_at(data, position), u32_at(data, position + 4))
            else {
                break;
            };
            if data_size != 0 {
                if let Some(slot) = found.iter_mut().find(|(id, _)| *id == entry.id) {
                    *slot = (entry.id, (data_rva, data_size));
                } else {
                    found.push((entry.id, (data_rva, data_size)));
                }
            }
            break;
        }
    }
    found
}

/// [`collect_resources`] for icon groups, keyed for lowest-first iteration.
///
/// The walk is the same; the container is a map because the caller tries
/// groups in ascending id order (`sorted(groups)`), and a map iterates that
/// way without a separate sort.
fn collect_group_resources(data: &[u8], base: usize, offset: u32) -> BTreeMap<u32, (u32, u32)> {
    collect_resources(data, base, offset).into_iter().collect()
}

/// Reassemble a real `.ico` file out of a `GROUP_ICON` and its images.
///
/// Port of `_build_ico` (`exe_icons.py:144-179`). A group entry whose image
/// is missing or empty is dropped, not fatal; the per-entry `_size` field is
/// trusted for nothing — the directory records each payload's actual length.
fn build_ico(group: &[u8], images: &[(u32, Vec<u8>)]) -> Option<Vec<u8>> {
    if group.len() < 6 {
        return None;
    }
    let reserved = u16_at(group, 0)?;
    let kind = u16_at(group, 2)?;
    let count = u16_at(group, 4)?;
    if reserved != 0 || kind != 1 || count == 0 || usize::from(count) > MAX_ICON_IMAGES {
        return None;
    }
    struct Entry<'a> {
        dimensions: [u8; 4],
        planes: u16,
        bits: u16,
        payload: &'a [u8],
    }
    let mut entries = Vec::new();
    let mut payload_bytes = 0usize;
    for index in 0..usize::from(count) {
        let position = 6 + index * 14;
        if position + 14 > group.len() {
            break;
        }
        let identifier = u16_at(group, position + 12)?;
        let Some(payload) = images
            .iter()
            .find(|(id, _)| *id == u32::from(identifier))
            .map(|(_, payload)| payload.as_slice())
        else {
            continue;
        };
        if payload.is_empty() {
            continue;
        }
        payload_bytes += payload.len();
        if payload_bytes > MAX_ICON_BYTES {
            break;
        }
        entries.push(Entry {
            dimensions: [
                group[position],
                group[position + 1],
                group[position + 2],
                group[position + 3],
            ],
            planes: u16_at(group, position + 4)?,
            bits: u16_at(group, position + 6)?,
            payload,
        });
    }
    if entries.is_empty() {
        return None;
    }

    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&u16::try_from(entries.len()).ok()?.to_le_bytes());
    let mut offset = u32::try_from(6 + 16 * entries.len()).ok()?;
    for entry in &entries {
        out.extend_from_slice(&entry.dimensions);
        out.extend_from_slice(&entry.planes.to_le_bytes());
        out.extend_from_slice(&entry.bits.to_le_bytes());
        out.extend_from_slice(&u32::try_from(entry.payload.len()).ok()?.to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += u32::try_from(entry.payload.len()).ok()?;
    }
    for entry in &entries {
        out.extend_from_slice(entry.payload);
    }
    Some(out)
}

/// Little-endian `u16` at `offset`, or `None` past the end of the data.
fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    let end = offset.checked_add(2)?;
    let bytes: [u8; 2] = data.get(offset..end)?.try_into().ok()?;
    Some(u16::from_le_bytes(bytes))
}

/// Little-endian `u32` at `offset`, or `None` past the end of the data.
fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    let end = offset.checked_add(4)?;
    let bytes: [u8; 4] = data.get(offset..end)?.try_into().ok()?;
    Some(u32::from_le_bytes(bytes))
}

/// Fixture builders shared with [`crate::covers`]' tests.
///
/// Port of the builders in `tests/test_exe_icons.py`, which the reference's
/// cover tests import (`tests/test_covers.py:21`) — the `.ico`-writing tests
/// need a real PE to read, and these build one. `pub(crate)` under `cfg(test)`
/// for exactly that import; production code never sees them.
#[cfg(test)]
pub(crate) mod builders {
    /// Where the resource section maps (`test_exe_icons.py:16`).
    pub(crate) const RSRC_RVA: u32 = 0x1000;
    /// Where the resource section's bytes sit in the file (`test_exe_icons.py:17`).
    pub(crate) const RSRC_FILE_OFFSET: usize = 0x400;
    const SUBDIRECTORY: u32 = 0x80000000;
    const LANGUAGE: u32 = 0x409;

    /// A real BITMAPINFOHEADER icon image of `side`×`side` 32-bit pixels.
    pub(crate) fn dib_icon(side: u32, fill: u8) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&40u32.to_le_bytes());
        out.extend_from_slice(&side.cast_signed().to_le_bytes());
        out.extend_from_slice(&side.saturating_mul(2).cast_signed().to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&side.saturating_mul(side).saturating_mul(4).to_le_bytes());
        out.extend_from_slice(&0i32.to_le_bytes());
        out.extend_from_slice(&0i32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for _ in 0..side.saturating_mul(side) {
            out.extend_from_slice(&[fill, fill, fill, 0xFF]);
        }
        let mask = side
            .saturating_add(31)
            .saturating_div(32)
            .saturating_mul(4)
            .saturating_mul(side) as usize;
        out.resize(out.len() + mask, 0);
        out
    }

    /// A GRPICONDIR over `(side, byte count, resource id)` entries.
    pub(crate) fn group_icon(entries: &[(u32, usize, u16)]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(
            &u16::try_from(entries.len())
                .expect("a fixture holds fewer than 65536 entries")
                .to_le_bytes(),
        );
        for (side, size, identifier) in entries {
            let byte = u8::try_from(side % 256).expect("side % 256 fits in a byte");
            out.extend_from_slice(&[byte, byte, 0, 0]);
            out.extend_from_slice(&1u16.to_le_bytes());
            out.extend_from_slice(&32u16.to_le_bytes());
            out.extend_from_slice(
                &u32::try_from(*size)
                    .expect("a fixture image is smaller than 4 GiB")
                    .to_le_bytes(),
            );
            out.extend_from_slice(&identifier.to_le_bytes());
        }
        out
    }

    fn directory(entries: &[(u32, usize, bool)]) -> Vec<u8> {
        // `IIHHHH`: characteristics, timestamp, major, minor, named count,
        // numbered count — 16 bytes, all zero but the last.
        let mut out = vec![0u8; 12];
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(
            &u16::try_from(entries.len())
                .expect("a fixture directory holds fewer than 65536 entries")
                .to_le_bytes(),
        );
        for (identifier, offset, is_directory) in entries {
            out.extend_from_slice(&identifier.to_le_bytes());
            let target = u32::try_from(*offset).expect("a fixture fits in 4 GiB")
                | if *is_directory { SUBDIRECTORY } else { 0 };
            out.extend_from_slice(&target.to_le_bytes());
        }
        out
    }

    fn put(blob: &mut [u8], offset: usize, data: &[u8]) {
        blob[offset..offset + data.len()].copy_from_slice(data);
    }

    /// A `.rsrc` section holding RT_ICON images and one RT_GROUP_ICON.
    pub(crate) fn resource_section(images: &[(u16, Vec<u8>)], group: &[u8]) -> Vec<u8> {
        resource_section_with_groups(images, &[(1, group.to_vec())])
    }

    /// A `.rsrc` section holding RT_ICON images and `(id, group)` icon groups.
    ///
    /// The reference's builder hard-codes a single group with id 1; this
    /// generalisation exists for the lowest-group-wins test, which needs two
    /// groups whose directory order disagrees with their numeric order.
    pub(crate) fn resource_section_with_groups(
        images: &[(u16, Vec<u8>)],
        groups: &[(u16, Vec<u8>)],
    ) -> Vec<u8> {
        let mut ids: Vec<u16> = images.iter().map(|(id, _)| *id).collect();
        ids.sort_unstable();
        let payload_of = |id: u16| {
            images
                .iter()
                .find(|(candidate, _)| *candidate == id)
                .map(|(_, payload)| payload)
                .expect("every sorted id came from the images")
        };

        let mut cursor = 0usize;
        let mut take = |size: usize| {
            let start = cursor;
            cursor += size;
            start
        };
        let root = take(16 + 2 * 8);
        let icon_type = take(16 + ids.len() * 8);
        let group_type = take(16 + groups.len() * 8);
        let icon_langs: Vec<usize> = ids.iter().map(|_| take(16 + 8)).collect();
        let group_langs: Vec<usize> = groups.iter().map(|_| take(16 + 8)).collect();
        let icon_data: Vec<usize> = ids.iter().map(|_| take(16)).collect();
        let group_data: Vec<usize> = groups.iter().map(|_| take(16)).collect();
        let icon_payloads: Vec<usize> = ids.iter().map(|id| take(payload_of(*id).len())).collect();
        let group_payloads: Vec<usize> =
            groups.iter().map(|(_, group)| take(group.len())).collect();

        let mut blob = vec![0u8; cursor];
        put(
            &mut blob,
            root,
            &directory(&[
                (super::RT_ICON, icon_type, true),
                (super::RT_GROUP_ICON, group_type, true),
            ]),
        );
        put(
            &mut blob,
            icon_type,
            &directory(
                &ids.iter()
                    .enumerate()
                    .map(|(n, id)| (u32::from(*id), icon_langs[n], true))
                    .collect::<Vec<_>>(),
            ),
        );
        put(
            &mut blob,
            group_type,
            &directory(
                &groups
                    .iter()
                    .enumerate()
                    .map(|(n, (id, _))| (u32::from(*id), group_langs[n], true))
                    .collect::<Vec<_>>(),
            ),
        );
        for (index, identifier) in ids.iter().enumerate() {
            put(
                &mut blob,
                icon_langs[index],
                &directory(&[(LANGUAGE, icon_data[index], false)]),
            );
            let mut entry = Vec::new();
            entry.extend_from_slice(&(RSRC_RVA + icon_payloads[index] as u32).to_le_bytes());
            entry.extend_from_slice(&(payload_of(*identifier).len() as u32).to_le_bytes());
            entry.extend_from_slice(&0u32.to_le_bytes());
            entry.extend_from_slice(&0u32.to_le_bytes());
            put(&mut blob, icon_data[index], &entry);
            put(&mut blob, icon_payloads[index], payload_of(*identifier));
        }
        for (index, (_, group)) in groups.iter().enumerate() {
            put(
                &mut blob,
                group_langs[index],
                &directory(&[(LANGUAGE, group_data[index], false)]),
            );
            let mut entry = Vec::new();
            entry.extend_from_slice(&(RSRC_RVA + group_payloads[index] as u32).to_le_bytes());
            entry.extend_from_slice(&(group.len() as u32).to_le_bytes());
            entry.extend_from_slice(&0u32.to_le_bytes());
            entry.extend_from_slice(&0u32.to_le_bytes());
            put(&mut blob, group_data[index], &entry);
            put(&mut blob, group_payloads[index], group);
        }
        blob
    }

    /// Where [`resource_section`] puts the first RT_ICON data entry.
    ///
    /// Mirrors the builder's own layout, so a test can corrupt one field
    /// without hard-coding an offset that drifts the moment the builder
    /// changes (`test_exe_icons.py:91-101`).
    pub(crate) fn icon_data_entry_offset(icon_count: usize) -> usize {
        let root = 16 + 2 * 8;
        let icon_type = 16 + icon_count * 8;
        let group_type = 16 + 8;
        let language_directories = (icon_count + 1) * (16 + 8);
        root + icon_type + group_type + language_directories
    }

    /// Wrap a resource section in the smallest PE the parser will accept.
    pub(crate) fn build_pe(section: &[u8], plus: bool, resource_size: Option<usize>) -> Vec<u8> {
        let magic = if plus { 0x20Bu16 } else { 0x10Bu16 };
        let optional_size = if plus { 240usize } else { 224usize };
        let (count_offset, directory_offset) = if plus { (108, 112) } else { (92, 96) };

        let mut dos = vec![0u8; 0x80];
        dos[0..2].copy_from_slice(b"MZ");
        dos[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());

        let mut coff = b"PE\x00\x00".to_vec();
        coff.extend_from_slice(&(if plus { 0x8664u16 } else { 0x14Cu16 }).to_le_bytes());
        coff.extend_from_slice(&1u16.to_le_bytes());
        coff.extend_from_slice(&0u32.to_le_bytes());
        coff.extend_from_slice(&0u32.to_le_bytes());
        coff.extend_from_slice(&0u32.to_le_bytes());
        coff.extend_from_slice(
            &u16::try_from(optional_size)
                .expect("the optional header fits in a u16")
                .to_le_bytes(),
        );
        coff.extend_from_slice(&0x0102u16.to_le_bytes());

        let mut optional = vec![0u8; optional_size];
        optional[0..2].copy_from_slice(&magic.to_le_bytes());
        optional[count_offset..count_offset + 4].copy_from_slice(&16u32.to_le_bytes());
        let directory = directory_offset + 16;
        optional[directory..directory + 4].copy_from_slice(&RSRC_RVA.to_le_bytes());
        let size = resource_size.unwrap_or(section.len()) as u32;
        optional[directory + 4..directory + 8].copy_from_slice(&size.to_le_bytes());

        let mut header = b".rsrc\x00\x00\x00".to_vec();
        header.extend_from_slice(&(section.len() as u32).to_le_bytes());
        header.extend_from_slice(&RSRC_RVA.to_le_bytes());
        header.extend_from_slice(&(section.len() as u32).to_le_bytes());
        header.extend_from_slice(&(RSRC_FILE_OFFSET as u32).to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(&0u32.to_le_bytes());
        header.extend_from_slice(&0u16.to_le_bytes());
        header.extend_from_slice(&0u16.to_le_bytes());
        header.extend_from_slice(&0x40000040u32.to_le_bytes());

        let mut body = dos;
        body.extend_from_slice(&coff);
        body.extend_from_slice(&optional);
        body.extend_from_slice(&header);
        body.resize(RSRC_FILE_OFFSET, 0);
        body.extend_from_slice(section);
        body
    }

    /// `(width, height, payload)` per image in an `.ico` file.
    pub(crate) fn read_ico(blob: &[u8]) -> Vec<(u8, u8, Vec<u8>)> {
        let count = u16::from_le_bytes([blob[4], blob[5]]);
        assert_eq!(&blob[0..4], &[0, 0, 1, 0]);
        let mut images = Vec::new();
        for index in 0..count as usize {
            let base = 6 + index * 16;
            let size = u32::from_le_bytes([
                blob[base + 8],
                blob[base + 9],
                blob[base + 10],
                blob[base + 11],
            ]) as usize;
            let offset = u32::from_le_bytes([
                blob[base + 12],
                blob[base + 13],
                blob[base + 14],
                blob[base + 15],
            ]) as usize;
            images.push((
                blob[base],
                blob[base + 1],
                blob[offset..offset + size].to_vec(),
            ));
        }
        images
    }
}

#[cfg(test)]
mod tests {
    use super::builders::*;
    use super::*;

    fn single_icon(fill: u8) -> (Vec<u8>, Vec<u8>) {
        let payload = dib_icon(16, fill);
        let section = resource_section(
            &[(1, payload.clone())],
            &group_icon(&[(16, payload.len(), 1)]),
        );
        (build_pe(&section, false, None), payload)
    }

    #[test]
    fn group_and_images_round_trip_into_an_ico() {
        let small = dib_icon(16, 0x11);
        let large = dib_icon(32, 0x22);
        let section = resource_section(
            &[(1, small.clone()), (2, large.clone())],
            &group_icon(&[(16, small.len(), 1), (32, large.len(), 2)]),
        );
        let blob = icon_bytes(&build_pe(&section, false, None)).expect("a valid icon");
        let images = read_ico(&blob);
        assert_eq!(
            images.iter().map(|(w, h, _)| (*w, *h)).collect::<Vec<_>>(),
            [(16, 16), (32, 32)]
        );
        assert_eq!(
            images.iter().map(|(_, _, p)| p).collect::<Vec<_>>(),
            [&small, &large]
        );
    }

    #[test]
    fn pe32_plus_executables_are_read_too() {
        let payload = dib_icon(16, 0x33);
        let section = resource_section(
            &[(1, payload.clone())],
            &group_icon(&[(16, payload.len(), 1)]),
        );
        let blob = icon_bytes(&build_pe(&section, true, None)).expect("a valid icon");
        assert_eq!(
            read_ico(&blob)
                .iter()
                .map(|(_, _, p)| p)
                .collect::<Vec<_>>(),
            [&payload]
        );
    }

    #[test]
    fn a_group_entry_with_no_image_is_dropped_not_fatal() {
        let payload = dib_icon(16, 0x44);
        let section = resource_section(
            &[(1, payload.clone())],
            &group_icon(&[(16, payload.len(), 1), (48, 999, 7)]),
        );
        let images = read_ico(&icon_bytes(&build_pe(&section, false, None)).expect("a valid icon"));
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].2, payload);
    }

    #[test]
    fn the_largest_available_size_survives() {
        let biggest = dib_icon(64, 0x55);
        let section = resource_section(
            &[(1, biggest.clone())],
            &group_icon(&[(0, biggest.len(), 1)]),
        );
        let images = read_ico(&icon_bytes(&build_pe(&section, false, None)).expect("a valid icon"));
        // A 256-wide icon records its width as 0; the byte is copied verbatim.
        assert_eq!((images[0].0, images[0].1), (0, 0));
        assert_eq!(images[0].2, biggest);
    }

    #[test]
    fn the_lowest_numbered_group_is_the_application_icon() {
        // Windows shows the lowest-numbered group, and the reference tries
        // `sorted(groups)` (`exe_icons.py:228`). The groups here are laid out
        // in directory order 2-then-1, so a port that tried directory order
        // would return the wrong artwork.
        let loser = dib_icon(16, 0xAA);
        let winner = dib_icon(16, 0xBB);
        let section = resource_section_with_groups(
            &[(1, loser.clone()), (2, winner.clone())],
            &[
                (2, group_icon(&[(16, loser.len(), 1)])),
                (1, group_icon(&[(16, winner.len(), 2)])),
            ],
        );
        let blob = icon_bytes(&build_pe(&section, false, None)).expect("a valid icon");
        let images = read_ico(&blob);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].2, winner);
    }

    #[test]
    fn a_file_that_is_not_a_pe_yields_no_icon() {
        assert_eq!(icon_bytes(&b"not an executable".repeat(8)), None);
    }

    #[test]
    fn a_truncated_executable_yields_no_icon() {
        let (full, _) = single_icon(0x66);
        assert_eq!(icon_bytes(&full[..RSRC_FILE_OFFSET + 8]), None);
    }

    #[test]
    fn a_resource_pointing_past_the_section_yields_no_icon() {
        let (full, _) = single_icon(0x77);
        let mut broken = full;
        let entry = RSRC_FILE_OFFSET + icon_data_entry_offset(1);
        broken[entry..entry + 4].copy_from_slice(&(RSRC_RVA + 0xFFFF).to_le_bytes());
        assert_eq!(icon_bytes(&broken), None);
    }

    #[test]
    fn an_executable_without_resources_yields_no_icon() {
        assert_eq!(icon_bytes(&build_pe(&[], false, Some(0))), None);
    }

    fn scratch_dir(label: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gh-exeicons-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a scratch directory");
        dir
    }

    #[test]
    fn reads_an_executable_off_disk() {
        let dir = scratch_dir("disk");
        let (exe, payload) = single_icon(0x88);
        let path = dir.join("app.exe");
        std::fs::write(&path, &exe).expect("write the fixture");
        let blob = extract_icon(&path).expect("an icon off disk");
        assert_eq!(
            read_ico(&blob)
                .iter()
                .map(|(_, _, p)| p)
                .collect::<Vec<_>>(),
            [&payload]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_and_empty_files_are_not_errors() {
        let dir = scratch_dir("missing");
        assert_eq!(extract_icon(&dir.join("nothing.exe")), None);
        let empty = dir.join("empty.exe");
        std::fs::write(&empty, b"").expect("write the empty fixture");
        assert_eq!(extract_icon(&empty), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_is_not_an_executable() {
        let dir = scratch_dir("dir");
        assert_eq!(extract_icon(&dir), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn headers_past_the_probe_still_resolve_through_the_full_read() {
        // `lfanew` past the 64 KiB probe: the bounded view cannot even find
        // the headers, so the full fallback does the whole job. Relocating
        // the headers moves the section bytes too, and the section header's
        // raw pointer follows them.
        let (small, payload) = single_icon(0x99);
        let lfanew = 0x20000usize;
        let headers_len = 24 + 224 + 40;
        let mut relocated = small[0x80..0x80 + headers_len].to_vec();
        let new_raw = lfanew + headers_len;
        relocated[headers_len - 20..headers_len - 16]
            .copy_from_slice(&(new_raw as u32).to_le_bytes());
        let mut file = small[..0x80].to_vec();
        file[0x3C..0x40].copy_from_slice(&(lfanew as u32).to_le_bytes());
        file.resize(lfanew, 0);
        file.extend_from_slice(&relocated);
        file.extend_from_slice(&small[RSRC_FILE_OFFSET..]);

        assert_eq!(icon_bytes(&file[..PROBE_BYTES]), None);
        let dir = scratch_dir("lfanew");
        let path = dir.join("far.exe");
        std::fs::write(&path, &file).expect("write the fixture");
        let blob = extract_icon(&path).expect("the fallback reads the whole file");
        assert_eq!(
            read_ico(&blob)
                .iter()
                .map(|(_, _, p)| p)
                .collect::<Vec<_>>(),
            [&payload]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resource_data_outside_the_bounded_view_resolves_through_the_full_read() {
        // The icon image lives in a second section past the probe while the
        // directories stay in `.rsrc`: the bounded view parses the headers
        // but cannot reach the payload, so only the full fallback finds it.
        // Inserting the second header shifts `.rsrc`'s raw bytes, and its raw
        // pointer follows them.
        const DATA_RVA: u32 = 0x2000;
        const PAYLOAD_OFF: usize = 96 * 1024;
        let payload = dib_icon(16, 0xAB);
        let section =
            resource_section(&[(1, vec![0u8; 8])], &group_icon(&[(16, payload.len(), 1)]));
        let mut pe = build_pe(&section, false, None);
        pe[0x80 + 6..0x80 + 8].copy_from_slice(&2u16.to_le_bytes());
        let rsrc_header = 0x80 + 24 + 224;
        let mut data_header = b".data\x00\x00\x00".to_vec();
        data_header.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        data_header.extend_from_slice(&DATA_RVA.to_le_bytes());
        data_header.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        data_header.extend_from_slice(&(PAYLOAD_OFF as u32).to_le_bytes());
        data_header.extend_from_slice(&0u32.to_le_bytes());
        data_header.extend_from_slice(&0u32.to_le_bytes());
        data_header.extend_from_slice(&0u16.to_le_bytes());
        data_header.extend_from_slice(&0u16.to_le_bytes());
        data_header.extend_from_slice(&0x40000040u32.to_le_bytes());
        pe.splice(
            rsrc_header + 40..rsrc_header + 40,
            data_header.iter().copied(),
        );
        let rsrc_raw = RSRC_FILE_OFFSET + 40;
        pe[rsrc_header + 20..rsrc_header + 24].copy_from_slice(&(rsrc_raw as u32).to_le_bytes());
        let entry = rsrc_raw + icon_data_entry_offset(1);
        pe[entry..entry + 4].copy_from_slice(&DATA_RVA.to_le_bytes());
        pe[entry + 4..entry + 8].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        pe.resize(PAYLOAD_OFF, 0);
        pe.extend_from_slice(&payload);

        assert_eq!(icon_bytes(&pe[..PROBE_BYTES]), None);
        let dir = scratch_dir("bound");
        let path = dir.join("split.exe");
        std::fs::write(&path, &pe).expect("write the fixture");
        let blob = extract_icon(&path).expect("the fallback reads the whole file");
        assert_eq!(
            read_ico(&blob)
                .iter()
                .map(|(_, _, p)| p)
                .collect::<Vec<_>>(),
            [&payload]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
