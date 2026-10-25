//! Dropdown placement (spec 7.13): a pure function of the tray icon rect,
//! monitor bounds and work area, unit-tested (D-040). The Win32 helper to
//! fetch the monitor and work area lives here too.

/// A rectangle in physical screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl ScreenRect {
    pub fn center_x(&self) -> i32 {
        self.x + self.w / 2
    }

    pub fn center_y(&self) -> i32 {
        self.y + self.h / 2
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }
}

/// The tray icon rect as the tray event reports it, in physical pixels.
pub type ClickRect = ScreenRect;

/// Distance between the window and the work area edge it sits on.
const GAP: i32 = 4;
/// Minimum gap (px) between bounds and work area that counts as a taskbar
/// edge; smaller values are DPI rounding noise.
const TASKBAR_MIN_GAP: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TaskbarEdge {
    Top,
    Bottom,
    Left,
    Right,
}

/// Infer the taskbar edge from where the work area differs from the monitor
/// bounds (Win10 allows any edge, spec 7.13). Falls back to `Bottom` when
/// the work area equals the bounds (no taskbar detected).
fn taskbar_edge(monitor: &ScreenRect, work: &ScreenRect) -> TaskbarEdge {
    let gaps = [
        (TaskbarEdge::Top, work.y - monitor.y),
        (TaskbarEdge::Bottom, monitor.bottom() - work.bottom()),
        (TaskbarEdge::Left, work.x - monitor.x),
        (TaskbarEdge::Right, monitor.right() - work.right()),
    ];
    gaps.into_iter()
        .max_by_key(|&(_, gap)| gap)
        .map_or(TaskbarEdge::Bottom, |(edge, gap)| {
            if gap > TASKBAR_MIN_GAP {
                edge
            } else {
                TaskbarEdge::Bottom
            }
        })
}

/// Where the dropdown window goes: on the inner side of the taskbar, as
/// close to the tray icon as possible, clamped inside the work area.
/// Returns the window's top-left corner in physical pixels.
pub fn place(
    click: &ClickRect,
    monitor: &ScreenRect,
    work: &ScreenRect,
    win: (i32, i32),
) -> (i32, i32) {
    let (win_w, win_h) = (win.0, win.1);
    let (mut x, mut y) = match taskbar_edge(monitor, work) {
        TaskbarEdge::Bottom => (click.center_x() - win_w / 2, work.bottom() - win_h - GAP),
        TaskbarEdge::Top => (click.center_x() - win_w / 2, work.y + GAP),
        TaskbarEdge::Left => (work.x + GAP, click.center_y() - win_h / 2),
        TaskbarEdge::Right => (work.right() - win_w - GAP, click.center_y() - win_h / 2),
    };
    // min/max instead of clamp: a work area smaller than the window makes
    // the lower bound exceed the upper one; then we prefer the top-left.
    x = x.min(work.right() - win_w - GAP).max(work.x + GAP);
    y = y.min(work.bottom() - win_h - GAP).max(work.y + GAP);
    (x, y)
}

/// Monitor bounds and work area containing the given point, in physical
/// pixels (Windows). Used by the tray click handler (spec 7.13).
#[cfg(windows)]
pub fn monitor_work_area_at(x: i32, y: i32) -> Option<(ScreenRect, ScreenRect)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    };

    let monitor: HMONITOR = unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_invalid() {
        return None;
    }
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: monitor handle from MonitorFromPoint; info is properly sized.
    if unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        let from = |r: windows::Win32::Foundation::RECT| ScreenRect {
            x: r.left,
            y: r.top,
            w: r.right - r.left,
            h: r.bottom - r.top,
        };
        Some((from(info.rcMonitor), from(info.rcWork)))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{ClickRect, ScreenRect, TaskbarEdge, place, taskbar_edge};

    const MONITOR: ScreenRect = ScreenRect {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
    };

    fn work(x: i32, y: i32, w: i32, h: i32) -> ScreenRect {
        ScreenRect { x, y, w, h }
    }

    fn click(x: i32, y: i32) -> ClickRect {
        // 40x40 tray icon at (x, y)
        ScreenRect { x, y, w: 40, h: 40 }
    }

    const WIN: (i32, i32) = (320, 440);

    #[test]
    fn bottom_taskbar_places_above_the_work_area_bottom() {
        let work = work(0, 0, 1920, 1040); // 40px taskbar at the bottom
        assert_eq!(taskbar_edge(&MONITOR, &work), TaskbarEdge::Bottom);
        let (x, y) = place(&click(960, 1045), &MONITOR, &work, WIN);
        assert_eq!(y, 1040 - 440 - 4);
        assert_eq!(x, 960 + 20 - 160); // centered on the icon's middle
    }

    #[test]
    fn top_taskbar_places_below_it() {
        let work = work(0, 48, 1920, 1032);
        assert_eq!(taskbar_edge(&MONITOR, &work), TaskbarEdge::Top);
        let (_, y) = place(&click(960, 10), &MONITOR, &work, WIN);
        assert_eq!(y, 48 + 4);
    }

    #[test]
    fn left_taskbar_places_to_its_right() {
        let work = work(48, 0, 1872, 1080);
        assert_eq!(taskbar_edge(&MONITOR, &work), TaskbarEdge::Left);
        let (x, _) = place(&click(10, 540), &MONITOR, &work, WIN);
        assert_eq!(x, 48 + 4);
    }

    #[test]
    fn right_taskbar_places_to_its_left() {
        let work = work(0, 0, 1872, 1080);
        assert_eq!(taskbar_edge(&MONITOR, &work), TaskbarEdge::Right);
        let (x, _) = place(&click(1890, 540), &MONITOR, &work, WIN);
        assert_eq!(x, 1872 - 320 - 4);
    }

    #[test]
    fn no_taskbar_gap_falls_back_to_bottom() {
        let work = MONITOR; // fullscreen app: no work area difference
        assert_eq!(taskbar_edge(&MONITOR, &work), TaskbarEdge::Bottom);
    }

    #[test]
    fn clamped_inside_the_work_area_at_the_right_edge() {
        let work = work(0, 0, 1920, 1040);
        // Tray icon far right: centering would push the window off-screen.
        let (x, _) = place(&click(1880, 1045), &MONITOR, &work, WIN);
        assert_eq!(x, 1920 - 320 - 4);
    }

    #[test]
    fn clamped_inside_the_work_area_at_the_left_edge() {
        let work = work(0, 0, 1920, 1040);
        let (x, _) = place(&click(2, 1045), &MONITOR, &work, WIN);
        assert_eq!(x, 4);
    }

    #[test]
    fn tiny_monitor_clamps_even_if_larger_than_window() {
        // Work area smaller than the window: the window is pinned to the
        // top-left of the work area (min/max chain prefers the inner edge).
        let work = work(0, 0, 200, 300);
        let (x, y) = place(&click(100, 290), &MONITOR, &work, (320, 440));
        assert_eq!(x, 4);
        assert_eq!(y, 4);
    }
}
