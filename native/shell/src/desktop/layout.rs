//! Grid spacing fills the usable desktop without adding overlapping partial edge cells.

pub const ORIGIN: f64 = 3.0;

/// Mutter may already have clipped the desktop to its work area; reserve the bar only once.
pub fn usable_height(actual: i32, monitor: i32, reserved: i32) -> f64 {
    actual.min((monitor - reserved).max(0)).max(0) as f64
}

/// Keep the cell count and saved grid coordinates, spreading spare pixels between cells.
/// `cell` remains the tile's reserved footprint; only the distance between origins grows.
pub fn axis(span: f64, cell: f64) -> (i32, f64) {
    let usable = (span - 2.0 * ORIGIN).max(0.0);
    let count = ((usable / cell).floor() as i32).max(1);
    let step = if count > 1 { (usable - cell) / f64::from(count - 1) } else { cell };
    (count, step)
}

/// Match the pixel-rounded origins used to draw tiles, including a fractional grid step.
pub fn cell_at(point: f64, step: f64, count: i32) -> i32 {
    let cell = (((point - ORIGIN) / step).floor() as i32).clamp(0, count - 1);
    if cell + 1 < count && (ORIGIN + f64::from(cell + 1) * step).round() <= point {
        cell + 1
    } else if cell > 0 && (ORIGIN + f64::from(cell) * step).round() > point {
        cell - 1
    } else {
        cell
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_cell_reaches_both_edges_without_overlapping() {
        for (width, height) in [(1920.0, 1080.0), (1366.0, 768.0), (1280.0, 720.0), (3840.0, 2160.0)] {
            for scale in [1.0, 1.25, 1.5, 2.0] {
                for (cw, ch) in [(76.0, 74.0), (76.0, 100.0), (112.0, 148.0)] {
                    for text_scale in [1.0, 1.5, 2.0] {
                        let cw: f64 = cw + (text_scale - 1.0) * 24.0;
                        let ch: f64 = ch + (text_scale - 1.0) * 32.0;
                        for bar in [0.0, 48.0] {
                            for (span, cell) in [(width / scale, cw.round()), (height / scale - bar, ch.round())] {
                                let (count, step) = axis(span, cell);
                                assert_eq!(count, (((span - 2.0 * ORIGIN) / cell).floor() as i32).max(1));
                                assert!(step >= cell - 0.000001, "adjacent tiles must not overlap");
                                if count > 1 {
                                    let last_end = ORIGIN + f64::from(count - 1) * step + cell;
                                    assert!((span - ORIGIN - last_end).abs() < 0.000001);
                                    // A drag to the final origin converts back to its saved integer cell.
                                    let last_start = ORIGIN + f64::from(count - 1) * step;
                                    assert_eq!(((last_start - ORIGIN) / step).round() as i32, count - 1);
                                    for index in 0..count {
                                        let pixel = (ORIGIN + f64::from(index) * step).round();
                                        assert_eq!(cell_at(pixel, step, count), index);
                                        if index > 0 {
                                            assert_eq!(cell_at(pixel - 1.0, step, count), index - 1);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn exact_fit_keeps_existing_spacing() {
        assert_eq!(axis(766.0, 76.0), (10, 76.0));
        assert_eq!(axis(1006.0, 100.0), (10, 100.0));
    }

    #[test]
    fn taskbar_is_reserved_once_for_full_or_constrained_desktops() {
        assert_eq!(usable_height(900, 900, 48), 852.0);
        assert_eq!(usable_height(852, 900, 48), 852.0);
        assert_eq!(usable_height(900, 900, 0), 900.0);
        assert_eq!(usable_height(852, 900, 0), 852.0);
        assert_eq!(usable_height(0, 900, 48), 0.0);
        assert_eq!(usable_height(30, 30, 48), 0.0);
    }

    #[test]
    fn small_workareas_keep_one_finite_cell() {
        for span in [0.0, 1.0, 6.0, 50.0, 106.0, 180.0] {
            let (count, step) = axis(span, 100.0);
            assert_eq!(count, 1);
            assert_eq!(step, 100.0);
        }
    }
}
