// Parts derived from CPython Lib/_pydatetime.py (Copyright (c) 2001 Python Software Foundation, PSF-2.0): see THIRD_PARTY_NOTICES.md and LICENSES/PSF-2.0.txt.
//! Python datetime.fromisoformat/timestamp semantics used by the XBEL reference.
//! Includes basic/week dates, arbitrary date/time separators and subsecond offsets.
use std::mem::MaybeUninit;

pub(super) fn iso_ms(text: Option<&str>) -> i64 {
    text.and_then(parse).unwrap_or(0)
}

fn number(chars: &[char]) -> Option<i64> {
    chars.iter().try_fold(0, |value, c| {
        c.is_ascii_digit()
            .then(|| value * 10 + (*c as i64 - '0' as i64))
    })
}
fn days(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let y = year - era * 400;
    let m = month + if month > 2 { -3 } else { 9 };
    era * 146097 + y * 365 + y / 4 - y / 100 + (153 * m + 2) / 5 + day - 1 - 719468
}
fn valid_date(year: i64, month: i64, day: i64) -> bool {
    if !(1..=9999).contains(&year) || !(1..=12).contains(&month) {
        return false;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max = match month {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=max).contains(&day)
}
fn separator(c: &[char]) -> Option<usize> {
    if c.len() < 7 {
        return None;
    }
    if c.len() == 7 {
        return Some(7);
    }
    if c[4] == '-' {
        if c[5] != 'W' {
            return Some(10);
        }
        if c.len() > 8 && c[8] == '-' {
            if c.len() == 9 {
                return None;
            }
            return Some(if c.get(10).is_some_and(char::is_ascii_digit) {
                8
            } else {
                10
            });
        }
        return Some(8);
    }
    if c[4] == 'W' {
        let mut index = 7;
        while c.get(index).is_some_and(char::is_ascii_digit) {
            index += 1;
        }
        return Some(if index < 9 {
            index
        } else if index % 2 == 0 {
            7
        } else {
            8
        });
    }
    Some(8)
}
fn date(c: &[char]) -> Option<i64> {
    if ![7, 8, 10].contains(&c.len()) {
        return None;
    }
    let year = number(c.get(..4)?)?;
    if !(1..=9999).contains(&year) {
        return None;
    }
    let dashed = c[4] == '-';
    let mut pos = 4 + usize::from(dashed);
    if c.get(pos) == Some(&'W') {
        pos += 1;
        let week = number(c.get(pos..pos + 2)?)?;
        pos += 2;
        let day = if pos == c.len() {
            1
        } else {
            if (c.get(pos) == Some(&'-')) != dashed {
                return None;
            }
            pos += usize::from(dashed);
            let day = number(c.get(pos..pos + 1)?)?;
            if pos + 1 != c.len() {
                return None;
            }
            day
        };
        if !(1..=53).contains(&week) || !(1..=7).contains(&day) {
            return None;
        }
        let jan4 = days(year, 1, 4);
        let monday = jan4 - (jan4 + 3).rem_euclid(7);
        let next_jan4 = days(year + 1, 1, 4);
        let next_monday = next_jan4 - (next_jan4 + 3).rem_euclid(7);
        let result = monday + 7 * (week - 1) + day - 1;
        if result >= next_monday || result < days(1, 1, 1) || result > days(9999, 12, 31) {
            return None;
        }
        return Some(result);
    }
    let month = number(c.get(pos..pos + 2)?)?;
    pos += 2;
    if (c.get(pos) == Some(&'-')) != dashed {
        return None;
    }
    pos += usize::from(dashed);
    let day = number(c.get(pos..pos + 2)?)?;
    if pos + 2 != c.len() || !valid_date(year, month, day) {
        return None;
    }
    Some(days(year, month, day))
}
fn hms(c: &[char]) -> Option<[i64; 4]> {
    let mut out = [0; 4];
    let mut pos = 0;
    let mut colons = false;
    for component in 0..3 {
        out[component] = number(c.get(pos..pos + 2)?)?;
        pos += 2;
        if component == 0 {
            colons = c.get(pos) == Some(&':');
        }
        if pos == c.len() || component == 2 {
            break;
        }
        if colons && c.get(pos) != Some(&':') {
            return None;
        }
        pos += usize::from(colons);
    }
    if pos < c.len() {
        if !matches!(c[pos], '.' | ',') {
            return None;
        }
        pos += 1;
        let fraction = &c[pos..];
        if fraction.is_empty() || !fraction.iter().all(char::is_ascii_digit) {
            return None;
        }
        let count = fraction.len().min(6);
        out[3] = number(&fraction[..count])? * 10_i64.pow(6 - count as u32);
    }
    Some(out)
}
fn local(epoch: i64) -> Option<i64> {
    let timestamp = epoch as libc::time_t;
    let mut tm = MaybeUninit::<libc::tm>::uninit();
    if unsafe { libc::localtime_r(&timestamp, tm.as_mut_ptr()) }.is_null() {
        return None;
    }
    let tm = unsafe { tm.assume_init() };
    let year = i64::from(tm.tm_year) + 1900;
    let month = i64::from(tm.tm_mon) + 1;
    let day = i64::from(tm.tm_mday);
    if !valid_date(year, month, day) {
        return None;
    }
    Some(
        days(year, month, day) * 86400
            + i64::from(tm.tm_hour) * 3600
            + i64::from(tm.tm_min) * 60
            + i64::from(tm.tm_sec),
    )
}
fn local_epoch(t: i64, local: impl Fn(i64) -> Option<i64>) -> Option<i64> {
    // Solve local(u)=t with the default fold=0, including ambiguous times and DST gaps.
    let a = local(t)? - t;
    let u1 = t - a;
    let t1 = local(u1)?;
    let b = if t1 == t {
        let previous = u1 - 86400;
        let b = local(previous)? - previous;
        if a == b {
            return Some(u1);
        }
        b
    } else {
        t1 - u1
    };
    let u2 = t - b;
    let t2 = local(u2)?;
    Some(if t2 == t {
        u2
    } else if t1 == t {
        u1
    } else {
        u1.max(u2)
    })
}
fn parse(text: &str) -> Option<i64> {
    let chars: Vec<char> = text.replace('Z', "+00:00").chars().collect();
    let split = separator(&chars)?;
    let mut day = date(chars.get(..split)?)?;
    let mut time = [0; 4];
    let mut offset = None;
    if split < chars.len() {
        let rest = chars.get(split + 1..)?;
        if rest.is_empty() {
            return None;
        }
        let tz = rest
            .iter()
            .position(|c| *c == '-')
            .or_else(|| rest.iter().position(|c| *c == '+'));
        time = hms(tz.map_or(rest, |index| &rest[..index]))?;
        if let Some(index) = tz {
            let zone = hms(&rest[index + 1..])?;
            let mut micros = (zone[0] * 3600 + zone[1] * 60 + zone[2]) * 1_000_000;
            // CPython treats an all-zero h/m/s offset as UTC, ignoring its fraction.
            if micros != 0 {
                micros += zone[3];
            }
            if micros >= 86400 * 1_000_000 {
                return None;
            }
            offset = Some(micros * if rest[index] == '-' { -1 } else { 1 });
        }
    }
    if time[0] == 24 && time[1..].iter().all(|value| *value == 0) {
        day += 1;
        time[0] = 0;
    }
    if day > days(9999, 12, 31) || time[0] > 23 || time[1] > 59 || time[2] > 59 {
        return None;
    }
    let seconds = day * 86400 + time[0] * 3600 + time[1] * 60 + time[2];
    let epoch = if let Some(offset) = offset {
        (seconds * 1_000_000 + time[3] - offset) as f64 / 1_000_000.0
    } else {
        local_epoch(seconds, local)? as f64 + time[3] as f64 / 1_000_000.0
    };
    Some((epoch * 1000.0) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn iso_formats_and_fractional_offsets_match_python_reference() {
        for (text, expected) in [
            ("2026-11-01T12:34:56Z", 1793536496000),
            ("2026-11-01T12:34:56.123456+02:30", 1793527496123),
            ("2026-W44-7T12:34:56+02:00", 1793529296000),
            ("20261101T123456+0000", 1793536496000),
            ("1969-12-31T23:59:59.999999Z", 0),
            ("1969-12-31T23:59:59.999Z", -1),
            ("2026-11-01T12:30:40,123456+02:30:05.5", 1793527234623),
            ("2026-11-01T24:00:00Z", 1793577600000),
            ("2026-11-01X12:34:56+02:00", 1793529296000),
            ("2026-11-01T12:34:56+00:99", 1793530556000),
            ("2026-11-01T12:34:56+00:00:00.5", 1793536496000),
            ("2026-W44T12:00+00:00", 1793016000000),
        ] {
            assert_eq!(iso_ms(Some(text)), expected, "{text}");
        }
    }
    #[test]
    fn malformed_dates_and_times_return_zero() {
        for text in [
            "",
            "nonsense",
            "2026-02-30T12:00:00Z",
            "2025-W53-1T00:00:00Z",
            "2026-11-01T",
            "2026-11-01T12.5",
            "2026-11-01T24:01:00Z",
            "2026-11-01T12:60:00Z",
            "2026-11-01T12:00:00+24:00",
            "0000-01-01T00:00:00Z",
        ] {
            assert_eq!(iso_ms(Some(text)), 0, "{text}");
        }
    }
    #[test]
    fn local_timestamp_solver_handles_fold_and_gap_without_global_tz_changes() {
        // At epoch 100000 the offset drops from 3600 to 0: local 101000 occurs twice.
        assert_eq!(
            local_epoch(101000, |u| Some(u + if u < 100000 { 3600 } else { 0 })),
            Some(97400)
        );
        // At the same instant an offset increase creates a gap; fold=0 picks its later epoch.
        assert_eq!(
            local_epoch(101000, |u| Some(u + if u < 100000 { 0 } else { 3600 })),
            Some(101000)
        );
    }
}
