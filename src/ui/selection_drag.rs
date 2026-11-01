use iced::Point;

/// Ignore tiny pointer jitter so a click does not start a measure selection.
const SELECTION_DRAG_THRESHOLD_PX: f32 = 12.0;

/// A press on the tablature: a click until the pointer moves far enough,
/// then a drag extending the measure selection.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SelectionDrag {
    #[default]
    Idle,
    /// Pressed on a beat, the pointer still near where it went down.
    Pressed { at: Point, focus_beat: usize },
    /// The pointer moved past the threshold: the selection follows it.
    Dragging { at: Point, focus_beat: usize },
}

/// How a press on the tablature ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DragRelease {
    /// Whether the pointer moved past the threshold, making it a drag.
    pub moved: bool,
    /// The beat pressed on.
    pub focus_beat: usize,
}

impl SelectionDrag {
    pub const fn press(at: Point, focus_beat: usize) -> Self {
        Self::Pressed { at, focus_beat }
    }

    pub const fn is_active(self) -> bool {
        !matches!(self, Self::Idle)
    }

    /// The pointer moved to `cursor`: whether the selection should follow
    /// it. Once past the threshold the press becomes a drag, but the
    /// selection only follows while the pointer stays clear of the start.
    pub fn move_to(&mut self, cursor: Point) -> bool {
        let (Self::Pressed { at, focus_beat } | Self::Dragging { at, focus_beat }) = *self else {
            return false;
        };
        let delta = cursor - at;
        if delta.x * delta.x + delta.y * delta.y
            < SELECTION_DRAG_THRESHOLD_PX * SELECTION_DRAG_THRESHOLD_PX
        {
            return false;
        }
        *self = Self::Dragging { at, focus_beat };
        true
    }

    /// The pointer was released: `None` when nothing was pressed.
    pub fn release(&mut self) -> Option<DragRelease> {
        let release = match *self {
            Self::Idle => None,
            Self::Pressed { focus_beat, .. } => Some(DragRelease {
                moved: false,
                focus_beat,
            }),
            Self::Dragging { focus_beat, .. } => Some(DragRelease {
                moved: true,
                focus_beat,
            }),
        };
        *self = Self::Idle;
        release
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: Point = Point::new(100.0, 100.0);

    #[test]
    fn a_press_without_movement_is_a_click() {
        let mut drag = SelectionDrag::press(ORIGIN, 3);
        assert!(drag.is_active());
        assert_eq!(
            drag.release(),
            Some(DragRelease {
                moved: false,
                focus_beat: 3
            })
        );
        assert!(!drag.is_active());
    }

    #[test]
    fn jitter_does_not_start_a_drag() {
        let mut drag = SelectionDrag::press(ORIGIN, 0);
        assert!(!drag.move_to(Point::new(105.0, 105.0)));
        assert!(!drag.release().unwrap().moved);
    }

    #[test]
    fn moving_past_the_threshold_starts_a_drag() {
        let mut drag = SelectionDrag::press(ORIGIN, 1);
        assert!(drag.move_to(Point::new(100.0, 112.0)));
        assert_eq!(
            drag.release(),
            Some(DragRelease {
                moved: true,
                focus_beat: 1
            })
        );
    }

    #[test]
    fn a_drag_brought_back_stays_a_drag_but_stops_following() {
        let mut drag = SelectionDrag::press(ORIGIN, 0);
        assert!(drag.move_to(Point::new(150.0, 100.0)));
        assert!(!drag.move_to(Point::new(101.0, 100.0)));
        assert!(drag.release().unwrap().moved);
    }

    #[test]
    fn nothing_happens_without_a_press() {
        let mut drag = SelectionDrag::default();
        assert!(!drag.move_to(Point::new(500.0, 500.0)));
        assert_eq!(drag.release(), None);
    }
}
