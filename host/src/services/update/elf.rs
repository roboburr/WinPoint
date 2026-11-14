//! The ELF `DT_NEEDED` parser for the compatibility check (design §6.2): 64-bit little-endian ELF only, the
//! `PT_DYNAMIC` segment's `DT_NEEDED` entries resolved through `DT_STRTAB` (a virtual address, mapped to a
//! file offset by the `PT_LOAD` segment that holds it), in the order `readelf -d` lists them. Reads only the
//! header, the program headers, the dynamic segment and the string table (no whole-file read: our binaries
//! are about 10 MB). Tested against `readelf -d` on the `BINS` of a build (unit test + tests/update/test_compat.sh).
use std::{fs::File, io, os::unix::fs::FileExt, path::Path};

const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const DT_NULL: u64 = 0;
const DT_NEEDED: u64 = 1;
const DT_STRTAB: u64 = 5;
const DT_STRSZ: u64 = 10;
/// Bounds for a file we did not write: at most 256 program headers and 1 MiB of dynamic section or strings.
const MAX_PHNUM: usize = 256;
const MAX_TABLE: u64 = 1 << 20;

fn bad(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("not a 64-bit LE ELF: {what}"),
    )
}
fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap_or_default())
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap_or_default())
}
fn read(file: &File, offset: u64, len: u64) -> io::Result<Vec<u8>> {
    if len > MAX_TABLE {
        return Err(bad("table too large"));
    }
    let mut buf = vec![0; len as usize];
    file.read_exact_at(&mut buf, offset)?;
    Ok(buf)
}

/// The NEEDED sonames of one ELF file, in the dynamic section's order.
pub(super) fn needed(path: &Path) -> io::Result<Vec<String>> {
    let file = File::open(path)?;
    let head = read(&file, 0, 64)?;
    if head[..4] != *b"\x7fELF" || head[4] != 2 || head[5] != 1 {
        return Err(bad("ident"));
    }
    let phoff = u64_at(&head, 0x20);
    let (phentsize, phnum) = (u16_at(&head, 0x36) as u64, u16_at(&head, 0x38) as usize);
    if phentsize < 56 || phnum > MAX_PHNUM {
        return Err(bad("program headers"));
    }
    // (type, offset, vaddr, filesz) of every program header
    let table = read(&file, phoff, phentsize * phnum as u64)?;
    let headers: Vec<(u32, u64, u64, u64)> = (0..phnum)
        .map(|i| i * phentsize as usize)
        .map(|at| {
            let field = |off: usize| u64_at(&table, at + off);
            (u32_at(&table, at), field(8), field(16), field(32))
        })
        .collect();
    let Some(&(_, dyn_off, _, dyn_size)) = headers.iter().find(|h| h.0 == PT_DYNAMIC) else {
        return Ok(Vec::new()); // a static binary: no NEEDED
    };
    let dynamic = read(&file, dyn_off, dyn_size)?;
    let (mut names, mut strtab, mut strsz) = (Vec::new(), None, None);
    for entry in dynamic.chunks_exact(16) {
        let (tag, value) = (u64_at(entry, 0), u64_at(entry, 8));
        match tag {
            DT_NULL => break,
            DT_NEEDED => names.push(value),
            DT_STRTAB => strtab = Some(value),
            DT_STRSZ => strsz = Some(value),
            _ => {}
        }
    }
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let (Some(strtab), Some(strsz)) = (strtab, strsz) else {
        return Err(bad("no string table"));
    };
    // the virtual address of the string table → its file offset, through the PT_LOAD segment that holds it
    let offset = headers
        .iter()
        .find(|h| h.0 == PT_LOAD && strtab >= h.2 && strtab - h.2 < h.3)
        .map(|h| strtab - h.2 + h.1)
        .ok_or_else(|| bad("string table outside every PT_LOAD"))?;
    let strings = read(&file, offset, strsz)?;
    names
        .into_iter()
        .map(|at| {
            let rest = strings
                .get(at as usize..)
                .ok_or_else(|| bad("name outside the string table"))?;
            let end = rest
                .iter()
                .position(|b| *b == 0)
                .ok_or_else(|| bad("unterminated name"))?;
            String::from_utf8(rest[..end].to_vec()).map_err(|_| bad("name is not UTF-8"))
        })
        .collect()
}

/// libc (with libm, the native programs' maths of the same package), libgcc_s and the dynamic loader are not
/// checked (design §6.2: GLIBC_2.39 is far below any new libc).
pub(super) fn skipped(soname: &str) -> bool {
    soname.starts_with("libc.so.")
        || soname.starts_with("libm.so.")
        || soname.starts_with("libgcc_s.so.")
        || soname.starts_with("ld-linux")
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    use std::{collections::BTreeMap, fs};

    /// A minimal ELF64 LE: one PT_LOAD at vaddr 0x400000 covering the file, one PT_DYNAMIC, a string table.
    pub(crate) fn tiny(names: &[&str], extra: &[(u64, u64)]) -> Vec<u8> {
        let mut strings = vec![0u8];
        let mut offsets = Vec::new();
        for name in names {
            offsets.push(strings.len() as u64);
            strings.extend_from_slice(name.as_bytes());
            strings.push(0);
        }
        let (phoff, dyn_off) = (64u64, 64 + 2 * 56);
        let mut dynamic: Vec<(u64, u64)> = offsets.iter().map(|o| (DT_NEEDED, *o)).collect();
        dynamic.extend_from_slice(extra);
        let str_off = dyn_off + (dynamic.len() as u64 + 3) * 16;
        dynamic.extend_from_slice(&[
            (DT_STRTAB, 0x400000 + str_off),
            (DT_STRSZ, strings.len() as u64),
            (DT_NULL, 0),
        ]);
        let total = str_off + strings.len() as u64;
        let mut b = vec![0u8; 64];
        b[..6].copy_from_slice(b"\x7fELF\x02\x01");
        b[0x20..0x28].copy_from_slice(&phoff.to_le_bytes());
        b[0x36..0x38].copy_from_slice(&56u16.to_le_bytes());
        b[0x38..0x3a].copy_from_slice(&2u16.to_le_bytes());
        let segments = [
            (PT_LOAD, 0, 0x400000, total),
            (
                PT_DYNAMIC,
                dyn_off,
                0x400000 + dyn_off,
                (dynamic.len() * 16) as u64,
            ),
        ];
        for (kind, off, vaddr, size) in segments {
            let mut ph = vec![0u8; 56];
            ph[..4].copy_from_slice(&kind.to_le_bytes());
            ph[8..16].copy_from_slice(&off.to_le_bytes());
            ph[16..24].copy_from_slice(&(vaddr as u64).to_le_bytes());
            ph[32..40].copy_from_slice(&size.to_le_bytes());
            b.extend(ph);
        }
        for (tag, value) in dynamic {
            b.extend(tag.to_le_bytes());
            b.extend(value.to_le_bytes());
        }
        b.extend(strings);
        b
    }
    fn scratch(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("w11u-elf-{name}-{}", crate::events::nonce()));
        fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn a_tiny_elf_lists_its_needed_in_order() {
        let names = ["libgtk-3.so.0", "libz.so.1", "libc.so.6"];
        let path = scratch("tiny", &tiny(&names, &[(12, 0x1000)]));
        assert_eq!(needed(&path).unwrap(), names);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn malformed_files_are_errors_not_panics() {
        let good = tiny(&["libz.so.1"], &[]);
        let mut cases: Vec<(&str, Vec<u8>)> = vec![
            ("empty", vec![]),
            ("short", good[..40].to_vec()),
            ("text", b"#!/bin/sh\n".repeat(8)),
        ];
        let mut be = good.clone();
        be[5] = 2;
        cases.push(("big-endian", be));
        let mut elf32 = good.clone();
        elf32[4] = 1;
        cases.push(("32-bit", elf32));
        let mut cut = good.clone();
        cut.truncate(good.len() - 4);
        cases.push(("truncated strings", cut));
        let mut many = good.clone();
        many[0x38..0x3a].copy_from_slice(&4000u16.to_le_bytes());
        cases.push(("too many headers", many));
        for (what, bytes) in cases {
            let path = scratch(what, &bytes);
            assert!(needed(&path).is_err(), "{what}");
            let _ = fs::remove_file(path);
        }
        // no PT_DYNAMIC (a static binary): no NEEDED, not an error
        let mut stat = good.clone();
        stat[64 + 56..64 + 60].copy_from_slice(&6u32.to_le_bytes());
        let path = scratch("static", &stat);
        assert_eq!(needed(&path).unwrap(), Vec::<String>::new());
        let _ = fs::remove_file(path);
    }

    #[test]
    fn the_skip_rule() {
        for name in [
            "libc.so.6",
            "libm.so.6",
            "libgcc_s.so.1",
            "ld-linux-x86-64.so.2",
        ] {
            assert!(skipped(name), "{name}");
        }
        for name in [
            "libcairo.so.2",
            "libgio-2.0.so.0",
            "libz.so.1",
            "libgtk-4.so.1",
        ] {
            assert!(!skipped(name), "{name}");
        }
    }

    /// The binaries of scripts/install.py `BINS` in a build directory (`W11_TEST_BUILD_DIR`, else
    /// tests/build/native-release) against `readelf -d`, recorded in tests/update/fixtures/compat/readelf-needed.txt
    /// (tests/update/test_compat.sh runs readelf again on every run and fails when that file is out of date; its
    /// --record-readelf writes it). A missing binary fails.
    #[test]
    fn our_binaries_equal_readelf() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let recorded =
            fs::read_to_string(root.join("tests/update/fixtures/compat/readelf-needed.txt"))
                .unwrap();
        let expected: BTreeMap<&str, Vec<&str>> = recorded
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .filter_map(|l| l.split_once(':'))
            .map(|(name, list)| (name, list.split_whitespace().collect()))
            .collect();
        let install = fs::read_to_string(root.join("scripts/install.py")).unwrap();
        let mut bins: Vec<&str> = install
            .split("\nBINS = (")
            .nth(1)
            .and_then(|t| t.split(')').next())
            .expect("install.py BINS")
            .split('"')
            .skip(1)
            .step_by(2)
            .collect();
        bins.sort_unstable();
        assert_eq!(
            expected.keys().copied().collect::<Vec<_>>(),
            bins,
            "the recording names every binary of BINS"
        );
        let dir = std::env::var_os("W11_TEST_BUILD_DIR").map_or_else(
            || root.join("tests/build/native-release"),
            std::path::PathBuf::from,
        );
        for (name, list) in &expected {
            let path = dir.join(name);
            let found = needed(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert_eq!(found, *list, "{name}");
        }
    }
}
