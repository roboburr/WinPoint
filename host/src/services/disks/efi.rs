use super::*;
pub(super) fn parse_load_option(raw: &[u8]) -> Option<Value> {
    if raw.len() < 10 {
        return None;
    }
    let data = &raw[4..];
    let attrs = u32::from_le_bytes(data[0..4].try_into().ok()?);
    let size = u16::from_le_bytes(data[4..6].try_into().ok()?) as usize;
    let mut end = 6;
    while end + 1 < data.len() && data[end..end + 2] != [0, 0] {
        end += 2
    }
    let name = String::from_utf16_lossy(
        &data[6..end]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect::<Vec<_>>(),
    )
    .trim()
    .to_owned();
    let fp = data
        .get(end + 2..(end + 2 + size).min(data.len()))
        .unwrap_or(&[]);
    let mut parts = vec![];
    let mut i = 0;
    while i + 4 <= fp.len() {
        let (ty, sub) = (fp[i], fp[i + 1]);
        let len = u16::from_le_bytes([fp[i + 2], fp[i + 3]]) as usize;
        if len < 4 || i + len > fp.len() {
            break;
        }
        if ty == 4 && sub == 1 && len >= 42 {
            let num = u32::from_le_bytes(fp[i + 4..i + 8].try_into().ok()?);
            let s = &fp[i + 24..i + 40];
            if fp[i + 41] == 2 {
                parts.push(format!(
                    "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                    u32::from_le_bytes(s[0..4].try_into().ok()?),
                    u16::from_le_bytes(s[4..6].try_into().ok()?),
                    u16::from_le_bytes(s[6..8].try_into().ok()?),
                    s[8],
                    s[9],
                    s[10],
                    s[11],
                    s[12],
                    s[13],
                    s[14],
                    s[15]
                ));
            } else if fp[i + 41] == 1 {
                parts.push(format!(
                    "{:08x}-{num:02x}",
                    u32::from_le_bytes(s[..4].try_into().ok()?)
                ));
            }
        }
        if ty == 0x7f && sub == 0xff {
            break;
        }
        i += len;
    }
    Some(json!({"name":name,"active":attrs&1!=0,"partuuids":parts}))
}
pub(super) fn entries() -> Vec<Value> {
    let root = PathBuf::from("/sys/firmware/efi/efivars");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return vec![];
    };
    let mut paths = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let n = p.file_name().unwrap_or_default().to_string_lossy();
            n.len() == 45
                && n.starts_with("Boot")
                && n.as_bytes()[4..8].iter().all(|b| b.is_ascii_hexdigit())
                && n.get(8..).is_some_and(|s| {
                    s.eq_ignore_ascii_case("-8be4df61-93ca-11d2-aa0d-00e098032b8c")
                })
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .into_iter()
        .filter_map(|p| {
            use std::io::Read;
            let mut raw = vec![];
            std::fs::File::open(p)
                .ok()?
                .take(65536)
                .read_to_end(&mut raw)
                .ok()?;
            parse_load_option(&raw).filter(|e| !arr(&e["partuuids"]).is_empty())
        })
        .collect()
}
