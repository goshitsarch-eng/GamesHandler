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
//!   takes no dependency for one, so [`extract_icon`] reads in stages: a
//!   header probe, a read through the resource section's raw bytes, a read
//!   through the furthest section's raw end when the first view clipped, and
//!   the whole file only for the two shapes no bound can name — headers past
//!   the probe, or a directory target outside every section (see
//!   [`bounded_attempt`]). An executable the headers already rule out — not
//!   a PE, or no resource directory — is decided from the probe and never
//!   buffered at all, which is the case a library scan hits constantly
//!   (`BUG-43`). Same observable contract (`None` for anything unreadable);
//!   the worst case is still [`MAX_EXECUTABLE_BYTES`], checked before any
//!   read, but it is now paid only where the file's own shape requires it.
//! - Reads are clamped to the bytes actually in hand, exactly as the
//!   reference's `data[offset : offset + size]` clamps (`exe_icons.py:212`) —
//!   `mmap` clamps a slice that runs past the end just as `bytes` does, so a
//!   payload whose declared size exceeds the file yields a *short* payload
//!   rather than an exception, and the group/image structure checks judge it
//!   on their own terms (`exe_icons.py:146`, `:155`). A short read is never
//!   refused: a truncated executable keeps whatever part of its icon survives
//!   (`BUG-41`).
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

/// One parse of one view of an executable.
///
/// `clipped` is the difference between "this view does not hold an icon" and
/// "this view was not asked the whole question": it is set whenever a read
/// was refused for running past the end of *these* bytes, which is the one
/// way a longer view can disagree. [`parse_view`]'s only caller that keeps it
/// is [`bounded_attempt`], where a clipped parse must fall back rather than
/// answer (`BUG-40`).
struct Parsed {
    icon: Option<Vec<u8>>,
    clipped: bool,
}

/// Return the primary icon of an in-memory PE image as `.ico` bytes.
///
/// Port of `icon_bytes` (`exe_icons.py:182-235`). `None` means "no usable
/// icon" — not a PE, truncated, no resources, no groups, no images, or a
/// group that assembles to nothing — never an error, because a cover lookup
/// that cannot read an icon just has no icon.
pub fn icon_bytes(data: &[u8]) -> Option<Vec<u8>> {
    parse_view(data).icon
}

/// [`icon_bytes`], reporting whether the parse ran out of bytes.
fn parse_view(data: &[u8]) -> Parsed {
    let mut clipped = false;
    let Some((sections, resource_rva, _resource_size)) =
        sections_and_resource_root(data, &mut clipped)
    else {
        // `clipped` carries the difference between "the headers ran past this
        // view" and "the headers this view does hold answer no" — a missing
        // resource directory is the file's own answer, not a short read, and
        // a longer view cannot overturn it (`BUG-43`).
        return Parsed {
            icon: None,
            clipped,
        };
    };
    let Some(base) = file_offset(&sections, resource_rva, 16) else {
        return Parsed {
            icon: None,
            clipped,
        };
    };

    // `{id: target}` over type directories that are subdirectories and
    // unnamed. A second entry with the same id overwrites the first, as the
    // reference's dict comprehension does.
    let mut icon_type = None;
    let mut group_type = None;
    for entry in directory_entries(data, base, 0, &mut clipped) {
        if entry.is_directory && !entry.is_named {
            if entry.id == RT_ICON {
                icon_type = Some(entry.target);
            } else if entry.id == RT_GROUP_ICON {
                group_type = Some(entry.target);
            }
        }
    }
    let (Some(icon_type), Some(group_type)) = (icon_type, group_type) else {
        return Parsed {
            icon: None,
            clipped,
        };
    };

    let groups = collect_group_resources(data, base, group_type, &mut clipped);
    let icons = collect_resources(data, base, icon_type, &mut clipped);
    if groups.is_empty() || icons.is_empty() {
        return Parsed {
            icon: None,
            clipped,
        };
    }

    let read = |entry: (u32, u32), clipped: &mut bool| -> Option<Vec<u8>> {
        let (rva, size) = entry;
        if size == 0 || usize::try_from(size).ok()? > MAX_ICON_BYTES {
            return None;
        }
        let size = size as usize;
        let offset = file_offset(&sections, rva, size)?;
        let end = offset.checked_add(size)?;
        if end > data.len() {
            // The reference slices, and the slice clamps (`exe_icons.py:212`)
            // — a short file yields a short payload, not an exception — so
            // every read in this view is silently short rather than refused.
            *clipped = true;
        }
        Some(
            data.get(offset..end.min(data.len()))
                .unwrap_or_default()
                .to_vec(),
        )
    };

    let mut images: Vec<(u32, Vec<u8>)> = Vec::new();
    let mut total = 0usize;
    for (identifier, entry) in &icons {
        let Some(payload) = read(*entry, &mut clipped) else {
            continue;
        };
        total += payload.len();
        if total > MAX_ICON_BYTES {
            break;
        }
        images.push((*identifier, payload));
    }
    if images.is_empty() {
        return Parsed {
            icon: None,
            clipped,
        };
    }

    // Windows shows the lowest-numbered icon group as the application icon,
    // and a map's values iterate in key order, so this is `sorted(groups)`.
    for entry in groups.values() {
        let Some(group) = read(*entry, &mut clipped) else {
            continue;
        };
        if let Some(ico) = build_ico(&group, &images) {
            return Parsed {
                icon: Some(ico),
                clipped,
            };
        }
    }
    Parsed {
        icon: None,
        clipped,
    }
}

/// Return *exe_path*'s embedded icon as `.ico` bytes, or `None`.
///
/// Port of `extract_icon` (`exe_icons.py:238-254`): a missing file, a
/// directory, anything under 64 bytes or over [`MAX_EXECUTABLE_BYTES`], and
/// anything the parser rejects all yield `None` rather than an error.
///
/// Files up to [`PROBE_BYTES`] are read whole — one syscall, one parse. Larger
/// files take the staged path: parse the headers from a probe, read through
/// the resource section's raw bytes, widen once to the furthest section's raw
/// end if that view clipped, and fall back to a full read only for what no
/// bound can name ([`bounded_attempt`]). The fallback matters: headers past
/// the probe, or a resource tree pointing outside every section, are legal,
/// and a bounded-only read would miss icons the reference finds — or, worse,
/// answer with a different one.
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
    match bounded_attempt(exe_path, len, &probe) {
        Attempt::Decided(icon) => icon,
        Attempt::NeedFull => icon_bytes(&read_prefix(exe_path, len)?),
    }
}

/// What a bounded pass could decide before paying for the whole file.
///
/// `Option<Option<Vec<u8>>>` would carry the same shape but read as an
/// accident, so the two answers get names.
enum Attempt {
    /// The probe — or a bound it implied — settled the question: an icon, or
    /// provably none.
    Decided(Option<Vec<u8>>),
    /// Only the whole file answers: the headers themselves ran past the
    /// probe, or the resource tree asked for bytes no section maps.
    NeedFull,
}

/// Try the resource section's raw bytes before paying for a full read.
///
/// Three outcomes rather than the old two. The headers decide the first:
/// when the probe holds them whole, "no resource directory" — and "not a PE"
/// — are the file's own answers, identical on any longer view, so they are
/// `Decided(None)` and the scan of an iconless executable costs one probe
/// rather than a half-gigabyte buffer. That was the live case the old
/// `Option` return could not express: `bounded_end` folded it into the same
/// `None` as "headers past the probe", and the fallback read ran
/// unconditionally (`BUG-43`).
///
/// When there *is* a resource directory, the first bound is that section's
/// raw end. A parse that clips it is not final — the group loop takes the
/// lowest-numbered group it can *read* (`exe_icons.py:228`), so a group the
/// view cannot reach hands its place to the next one and the bounded parse
/// returns a real, well-formed, wrong icon (`BUG-40`). Clipping therefore
/// widens once, to the furthest raw end any section claims: every payload
/// read is resolved by [`file_offset`], which only returns offsets inside a
/// section's raw bytes, so that bound covers them all. A parse that *still*
/// clips is walking a directory target outside every section — legal, since
/// directory offsets are added to the resource root unchecked — and only the
/// whole file answers it.
fn bounded_attempt(exe_path: &Path, len: usize, probe: &[u8]) -> Attempt {
    let mut clipped = false;
    let Some((sections, resource_rva, _size)) = sections_and_resource_root(probe, &mut clipped)
    else {
        return if clipped {
            Attempt::NeedFull
        } else {
            Attempt::Decided(None)
        };
    };

    // The resource RVA may map to no section at all; the probe still answers,
    // because `file_offset` fails on those same bytes in any view.
    let end = resource_section_end(&sections, resource_rva)
        .unwrap_or(probe.len())
        .min(len);
    let mut parsed = match read_to(exe_path, end, probe) {
        Some(parsed) => parsed,
        None => return Attempt::Decided(None),
    };
    if !parsed.clipped {
        return Attempt::Decided(parsed.icon);
    }

    let wider = sections_end(&sections).map_or(len, |end| end.min(len));
    if wider > end {
        parsed = match read_to(exe_path, wider, probe) {
            Some(parsed) => parsed,
            None => return Attempt::Decided(None),
        };
        if !parsed.clipped {
            return Attempt::Decided(parsed.icon);
        }
    }
    Attempt::NeedFull
}

/// Parse the probe when it already covers `end`, else read that far.
fn read_to(exe_path: &Path, end: usize, probe: &[u8]) -> Option<Parsed> {
    if end <= probe.len() {
        return Some(parse_view(probe));
    }
    // A failed read means the file shrank past `end`; the whole-file read
    // would fail the same way, so `None` is decided rather than deferred.
    read_prefix(exe_path, end).map(|view| parse_view(&view))
}

/// End offset of a bounded read: through the resource section's raw bytes.
///
/// `None` when the resource RVA maps to no section — the probe then answers
/// by itself, since `file_offset` fails on the same header bytes in any view.
fn resource_section_end(sections: &[Section], resource_rva: u32) -> Option<usize> {
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

/// End offset covering every section's raw bytes — the furthest a payload
/// read can reach, because [`file_offset`] only resolves inside them.
fn sections_end(sections: &[Section]) -> Option<usize> {
    sections
        .iter()
        .map(|section| u64::from(section.raw_pointer) + u64::from(section.raw_size))
        .max()
        .and_then(|end| usize::try_from(end).ok())
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
///
/// `clipped` is set when the section table is cut short by the end of `data`
/// — the reference simply walks fewer sections there, so this is a read the
/// view could not serve rather than a verdict about the file (`BUG-40`).
fn sections_and_resource_root(data: &[u8], clipped: &mut bool) -> Option<(Vec<Section>, u32, u32)> {
    // Every `None` here is one of two different answers, and `clipped` is how
    // the caller tells them apart: an exit on a read the view could not serve
    // is "ask again with more bytes", while an exit on bytes the view *did*
    // hold is the file's own answer, identical on any longer view — a prefix
    // read and the whole file share their header bytes. The old code folded
    // both into a bare `None`, which is how `extract_icon` came to buffer a
    // whole executable just to re-derive "no resource directory" (BUG-43).
    if data.len() < 64 {
        *clipped = true;
        return None;
    }
    if data.get(0..2)? != b"MZ" {
        return None;
    }
    let lfanew = u32_at(data, 0x3C)?;
    if lfanew == 0 {
        return None;
    }
    let lfanew = lfanew as usize;
    if lfanew.checked_add(24).is_none_or(|end| end > data.len()) {
        *clipped = true;
        return None;
    }
    if data.get(lfanew..lfanew.checked_add(4)?)? != b"PE\x00\x00" {
        return None;
    }

    let section_count = u16_at(data, lfanew.checked_add(6)?)?;
    let optional_size = u16_at(data, lfanew.checked_add(20)?)?;
    let optional = lfanew.checked_add(24)?;
    if optional_size < 2 {
        return None;
    }
    if optional
        .checked_add(optional_size as usize)
        .is_none_or(|end| end > data.len())
    {
        *clipped = true;
        return None;
    }
    let magic = u16_at(data, optional)?;
    let (count_offset, directory_offset) = match magic {
        PE32_MAGIC => (92usize, 96usize),
        PE32PLUS_MAGIC => (108usize, 112usize),
        _ => return None,
    };
    let directories_end = optional
        .checked_add(directory_offset)
        .and_then(|base| base.checked_add(8 * (RESOURCE_DIRECTORY_INDEX + 1)));
    if directories_end.is_none_or(|end| end > data.len()) {
        *clipped = true;
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
            *clipped = true;
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
/// Port of `_file_offset` (`exe_icons.py:86-94`), checks and all: the
/// reference refuses an address no section maps and one whose length runs
/// past the section's *raw* bytes, and so does this. It deliberately does
/// **not** refuse a read that runs past the end of the bytes in hand — the
/// reference does not either, and returning `None` there loses an icon the
/// reference recovers from a truncated file (`BUG-41`). The caller clamps
/// instead, which is what `data[offset : offset + size]` does
/// (`exe_icons.py:212`). Arithmetic is `u64` throughout so a hostile header
/// cannot wrap a containment check.
fn file_offset(sections: &[Section], rva: u32, length: usize) -> Option<usize> {
    let rva = u64::from(rva);
    let length = length as u64;
    for section in sections {
        let start = u64::from(section.virtual_address);
        if rva >= start && rva < start + u64::from(section.span) {
            let delta = rva - start;
            if delta + length > u64::from(section.raw_size) {
                return None;
            }
            return usize::try_from(u64::from(section.raw_pointer) + delta).ok();
        }
    }
    None
}

/// `(id, target, is_directory, is_named)` for one resource directory.
///
/// Port of `_directory_entries` (`exe_icons.py:97-118`). `clipped` is set
/// where the reference's own length guards stop the walk early: it read fewer
/// entries than the directory declares, which a longer view might not.
fn directory_entries(
    data: &[u8],
    base: usize,
    offset: u32,
    clipped: &mut bool,
) -> Vec<DirectoryEntry> {
    let Some(start) = base.checked_add(offset as usize) else {
        *clipped = true;
        return Vec::new();
    };
    if start.checked_add(16).is_none_or(|end| end > data.len()) {
        *clipped = true;
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
            *clipped = true;
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
fn collect_resources(
    data: &[u8],
    base: usize,
    offset: u32,
    clipped: &mut bool,
) -> Vec<(u32, (u32, u32))> {
    let mut found: Vec<(u32, (u32, u32))> = Vec::new();
    for entry in directory_entries(data, base, offset, clipped) {
        if entry.is_named || !entry.is_directory {
            continue;
        }
        for leaf in directory_entries(data, base, entry.target, clipped) {
            if leaf.is_directory {
                continue;
            }
            let Some(position) = base.checked_add(leaf.target as usize) else {
                *clipped = true;
                break;
            };
            if position.checked_add(16).is_none_or(|end| end > data.len()) {
                *clipped = true;
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
fn collect_group_resources(
    data: &[u8],
    base: usize,
    offset: u32,
    clipped: &mut bool,
) -> BTreeMap<u32, (u32, u32)> {
    collect_resources(data, base, offset, clipped)
        .into_iter()
        .collect()
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
    pub(crate) const LANGUAGE: u32 = 0x409;

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

    pub(crate) fn directory(entries: &[(u32, usize, bool)]) -> Vec<u8> {
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

    pub(crate) fn put(blob: &mut [u8], offset: usize, data: &[u8]) {
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

    /// Where [`resource_section_with_groups`] puts group `index`'s data entry.
    ///
    /// [`icon_data_entry_offset`]'s sibling, and for the same reason: the
    /// group's payload address is the field `BUG-40` turns on, and a
    /// hard-coded offset would drift with the builder.
    pub(crate) fn group_data_entry_offset(
        icon_count: usize,
        group_count: usize,
        index: usize,
    ) -> usize {
        let root = 16 + 2 * 8;
        let icon_type = 16 + icon_count * 8;
        let group_type = 16 + group_count * 8;
        let icon_langs = icon_count * (16 + 8);
        let group_langs = group_count * (16 + 8);
        let icon_data = icon_count * 16;
        root + icon_type + group_type + icon_langs + group_langs + icon_data + index * 16
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
    fn a_payload_cut_short_by_the_end_of_the_file_is_clamped_not_dropped() {
        // `data[offset : offset + size]` clamps (`exe_icons.py:212`), so a
        // file ending four bytes into the group's *last entry* still has a
        // group: the entries that fit are assembled and the rest are gone.
        // Refusing the read instead loses the image entirely.
        //
        // This is the reference's own two-image fixture
        // (`tests/test_exe_icons.py:53-59`) with the last four bytes cut,
        // which is where its group payload sits. Measured against
        // `gamehandler/exe_icons.py` on these exact bytes: `icon_bytes`
        // returns a 1150-byte ICO holding one 16×16 image — the first entry's
        // — and `extract_icon` returns the same 1150 bytes.
        let small = dib_icon(16, 0x11);
        let large = dib_icon(32, 0x22);
        let section = resource_section(
            &[(1, small.clone()), (2, large.clone())],
            &group_icon(&[(16, small.len(), 1), (32, large.len(), 2)]),
        );
        let mut full = build_pe(&section, false, None);
        full.truncate(full.len() - 4);

        let blob = icon_bytes(&full).expect("the entry that survives the cut");
        let images = read_ico(&blob);
        assert_eq!(images.len(), 1, "the second group entry does not fit");
        assert_eq!(images[0].2, small);
    }

    #[test]
    fn a_truncated_file_read_off_disk_keeps_the_same_image() {
        // The same property through [`extract_icon`], which is the path the
        // cover lookup actually takes (`covers.rs`'s `save_exe_icon`): the
        // file is under [`PROBE_BYTES`], so it is read whole and the clamp
        // has to happen inside the parse rather than at the read.
        let small = dib_icon(16, 0x11);
        let large = dib_icon(32, 0x22);
        let section = resource_section(
            &[(1, small.clone()), (2, large.clone())],
            &group_icon(&[(16, small.len(), 1), (32, large.len(), 2)]),
        );
        let mut full = build_pe(&section, false, None);
        full.truncate(full.len() - 4);

        let dir = scratch_dir("truncated-disk");
        let path = dir.join("cut.exe");
        std::fs::write(&path, &full).expect("write the fixture");
        let blob = extract_icon(&path).expect("the entry that survives the cut");
        assert_eq!(
            read_ico(&blob)
                .iter()
                .map(|(_, _, p)| p)
                .collect::<Vec<_>>(),
            [&small]
        );
        let _ = std::fs::remove_dir_all(&dir);
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
    fn resource_data_in_another_section_resolves_through_the_widened_bound() {
        // The icon image lives in a second section past the probe while the
        // directories stay in `.rsrc`: the resource-section view parses the
        // headers but cannot reach the payload, so it clips — and the widened
        // bound (the furthest section's raw end) finds it without paying for
        // the whole file (`BUG-43`). Inserting the second header shifts
        // `.rsrc`'s raw bytes, and its raw pointer follows them.
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
        let blob = extract_icon(&path).expect("the widened bound reaches .data");
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
    fn a_group_the_bounded_view_cannot_read_does_not_hand_the_icon_to_the_next() {
        // `BUG-40`'s shape, and the reason "a bounded parse that yields an
        // icon is final" was false. Group 1's payload lives in a second
        // section, past both `.rsrc`'s raw bytes and the probe; group 2's
        // stays in `.rsrc`. The group loop takes the lowest-numbered group it
        // can *read* (`exe_icons.py:228`), so the bounded view does not
        // report a *missing* icon — it reports group 2's, which is a real,
        // well-formed, wrong one.
        //
        // Measured against `gamehandler/exe_icons.py` on these exact bytes:
        // `extract_icon` returns group 1's image (the 0xAA fill, which is
        // what Windows shows), while `icon_bytes` over the first [`PROBE_BYTES`]
        // returns group 2's (0xBB) — the wrong answer the bounded parse would
        // have given had it been allowed to settle the question. Both are
        // asserted below, so neither a parse that refuses everything nor one
        // that answers from the wrong group can pass.
        const DATA_RVA: u32 = 0x2000;
        const PAYLOAD_OFF: usize = 96 * 1024;
        let first = dib_icon(16, 0xAA);
        let second = dib_icon(16, 0xBB);
        let group1 = group_icon(&[(16, first.len(), 1)]);
        let group2 = group_icon(&[(16, second.len(), 2)]);
        let section = resource_section_with_groups(
            &[(1, first.clone()), (2, second.clone())],
            &[(1, group1.clone()), (2, group2)],
        );
        let mut pe = build_pe(&section, false, None);
        pe[0x80 + 6..0x80 + 8].copy_from_slice(&2u16.to_le_bytes());
        let rsrc_header = 0x80 + 24 + 224;
        let mut data_header = b".data\x00\x00\x00".to_vec();
        data_header.extend_from_slice(&(group1.len() as u32).to_le_bytes());
        data_header.extend_from_slice(&DATA_RVA.to_le_bytes());
        data_header.extend_from_slice(&(group1.len() as u32).to_le_bytes());
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
        let entry = rsrc_raw + group_data_entry_offset(2, 2, 0);
        pe[entry..entry + 4].copy_from_slice(&DATA_RVA.to_le_bytes());
        pe[entry + 4..entry + 8].copy_from_slice(&(group1.len() as u32).to_le_bytes());
        pe.resize(PAYLOAD_OFF, 0);
        pe.extend_from_slice(&group1);

        let bounded = read_ico(&icon_bytes(&pe[..PROBE_BYTES]).expect("group 2's icon"));
        assert_eq!(bounded.len(), 1);
        assert_eq!(bounded[0].2, second);

        let dir = scratch_dir("two-groups");
        let path = dir.join("split.exe");
        std::fs::write(&path, &pe).expect("write the fixture");
        let blob = extract_icon(&path).expect("group 1's icon");
        let images = read_ico(&blob);
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].2, first);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_header_the_view_cannot_hold_is_clipped_not_negative() {
        // The split `BUG-43` depends on: a header read the view cannot serve
        // is `clipped` — a longer view may still answer — while a negative
        // read of header bytes the view *does* hold is the file's own answer
        // and no view overturns it.
        let (exe, _) = single_icon(0x66);
        let mut clipped = false;
        // The optional header is cut mid-field: `optional + optional_size`
        // runs past this prefix.
        assert!(sections_and_resource_root(&exe[..0x80 + 24 + 10], &mut clipped).is_none());
        assert!(
            clipped,
            "a truncated optional header is a short view, not an answer"
        );
        // "No resource directory" is complete in any view that holds the
        // headers — `build_pe(.., Some(0))` writes `resource_size = 0`.
        let iconless = build_pe(&[], false, Some(0));
        let mut clipped = false;
        assert!(sections_and_resource_root(&iconless, &mut clipped).is_none());
        assert!(
            !clipped,
            "no resource directory is the file's answer, not a short view"
        );
    }

    #[test]
    fn an_iconless_executable_is_decided_from_the_probe_not_the_full_read() {
        // The case `BUG-43` is about: a >64 KiB executable whose headers
        // parse cleanly and say *no resource directory* can never hold an
        // icon — yet the old code buffered the whole file anyway, because
        // `bounded_end` folded that answer into the same `None` as "headers
        // past the probe". `Decided(None)` is the pin: it is reachable only
        // because the header parse now tells a short view from a negative
        // answer, and under the old shape this branch could not exist.
        let mut pe = build_pe(&[], false, Some(0));
        pe.resize(PROBE_BYTES + 4096, 0);

        let dir = scratch_dir("iconless");
        let path = dir.join("iconless.exe");
        std::fs::write(&path, &pe).expect("write the fixture");
        let probe = read_prefix(&path, PROBE_BYTES).expect("the probe read");
        assert!(
            matches!(
                bounded_attempt(&path, pe.len(), &probe),
                Attempt::Decided(None)
            ),
            "an iconless header set is the file's own answer — no full read"
        );
        assert_eq!(extract_icon(&path), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_resource_tree_in_the_overlay_still_resolves_through_the_full_read() {
        // The one shape no section bound covers: a resource *directory*'s
        // target is added to the root's file offset unchecked
        // (`collect_resources`), so it can point into overlay data past every
        // section. Both bounded views clip on it, `sections_end` adds nothing
        // — `.rsrc` is the only section — and only the whole file holds the
        // subdirectories. This is what `NeedFull` is still for.
        const ICON_DIR: usize = 0x20000;
        const ICON_LANG: usize = 0x20100;
        const ICON_DATA: usize = 0x20200;
        const GROUP_DIR: usize = 0x20300;
        const GROUP_LANG: usize = 0x20400;
        const GROUP_DATA: usize = 0x20500;

        let payload = dib_icon(16, 0xCC);
        let group = group_icon(&[(16, payload.len(), 1)]);
        // `.rsrc` holds the root directory — whose two targets live in the
        // overlay — plus the icon and group payloads.
        let icon_at = 16 + 2 * 8;
        let group_at = icon_at + payload.len();
        let mut section = directory(&[
            (super::RT_ICON, ICON_DIR, true),
            (super::RT_GROUP_ICON, GROUP_DIR, true),
        ]);
        section.extend_from_slice(&payload);
        section.extend_from_slice(&group);

        let mut file = build_pe(&section, false, None);
        let data_entry = |rva: u32, size: usize| {
            let mut entry = Vec::new();
            entry.extend_from_slice(&rva.to_le_bytes());
            entry.extend_from_slice(&(size as u32).to_le_bytes());
            entry.extend_from_slice(&0u32.to_le_bytes());
            entry.extend_from_slice(&0u32.to_le_bytes());
            entry
        };
        file.resize(RSRC_FILE_OFFSET + GROUP_DATA + 16, 0);
        put(
            &mut file,
            RSRC_FILE_OFFSET + ICON_DIR,
            &directory(&[(1, ICON_LANG, true)]),
        );
        put(
            &mut file,
            RSRC_FILE_OFFSET + ICON_LANG,
            &directory(&[(LANGUAGE, ICON_DATA, false)]),
        );
        put(
            &mut file,
            RSRC_FILE_OFFSET + ICON_DATA,
            &data_entry(RSRC_RVA + icon_at as u32, payload.len()),
        );
        put(
            &mut file,
            RSRC_FILE_OFFSET + GROUP_DIR,
            &directory(&[(1, GROUP_LANG, true)]),
        );
        put(
            &mut file,
            RSRC_FILE_OFFSET + GROUP_LANG,
            &directory(&[(LANGUAGE, GROUP_DATA, false)]),
        );
        put(
            &mut file,
            RSRC_FILE_OFFSET + GROUP_DATA,
            &data_entry(RSRC_RVA + group_at as u32, group.len()),
        );

        // The bounded pass must clip twice and defer — the directories are at
        // 128 KiB+, past both the probe and every section's raw end.
        let dir = scratch_dir("overlay");
        let path = dir.join("overlay.exe");
        std::fs::write(&path, &file).expect("write the fixture");
        let probe = read_prefix(&path, PROBE_BYTES).expect("the probe read");
        assert!(
            matches!(
                bounded_attempt(&path, file.len(), &probe),
                Attempt::NeedFull
            ),
            "a directory tree outside every section is the full read's case"
        );
        let blob = extract_icon(&path).expect("the full read reaches the overlay");
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
    fn a_hostile_executable_is_never_a_panic() {
        // The parser walks offsets from data nobody in this process wrote, so
        // the contract for every one of them is a value. Structured rather
        // than random: **every** prefix length of a real executable is tried,
        // because that is the boundary the EOF check lived at and the one a
        // hand-written bounds check gets wrong, and each 4-byte header field
        // the parser follows is poisoned in turn. The corpus is checked for
        // *reach* before the panic-freedom is claimed — a corpus of inputs
        // that all fail the `MZ` test would pass this test while touching
        // nothing — by requiring that it produces both an icon and a refusal.
        let small = dib_icon(16, 0x11);
        let large = dib_icon(32, 0x22);
        let section = resource_section(
            &[(1, small.clone()), (2, large.clone())],
            &group_icon(&[(16, small.len(), 1), (32, large.len(), 2)]),
        );
        let pe = build_pe(&section, false, None);

        let mut corpus: Vec<Vec<u8>> = (0..=pe.len()).map(|end| pe[..end].to_vec()).collect();
        // `lfanew`, section count, the size/count of every optional-header
        // field the walk reads, the resource directory entry, and the first
        // section header's four words.
        for offset in [
            0x3C, 0x86, 0x94, 0x98, 0x9C, 0xA0, 0xE0, 0xE4, 0xE8, 0xEC, 0x180, 0x184, 0x188, 0x18C,
            0x190,
        ] {
            for value in [0u32, 1, 0x00FF_FFFF, 0x7FFF_FFFF, 0xFFFF_FFFF] {
                let mut broken = pe.clone();
                if offset + 4 <= broken.len() {
                    broken[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
                    corpus.push(broken);
                }
            }
        }
        // Deterministic single-byte corruption over the whole fixture
        // (xorshift, so no dependency and no flaky case).
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        for _ in 0..1024 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let mut broken = pe.clone();
            let index = (state as usize) % broken.len();
            broken[index] = (state >> 32) as u8;
            corpus.push(broken);
        }
        assert!(corpus.len() > 5_000, "the corpus is {}", corpus.len());

        let parsed: Vec<Option<Vec<u8>>> = corpus.iter().map(|case| icon_bytes(case)).collect();
        let icons = parsed.iter().filter(|icon| icon.is_some()).count();
        // Measured: 7,758 cases, 1,075 of them yielding an icon and 6,683
        // refused — so neither assertion below is satisfied by an empty corpus
        // and the panic-freedom above is claimed over inputs that reached the
        // walk rather than the `MZ` test.
        assert!(icons > 0, "no case in the corpus produced an icon");
        assert!(icons < corpus.len(), "no case in the corpus was refused");

        // The same corpus through the file path the cover lookup uses.
        let dir = scratch_dir("hostile");
        for (index, case) in corpus.iter().enumerate() {
            let path = dir.join(format!("case-{index}.exe"));
            std::fs::write(&path, case).expect("write the case");
            let _ = extract_icon(&path);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
