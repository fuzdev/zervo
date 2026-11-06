//! Pointer, wheel, touch, cursor, and theme mapping between winit and Servo.
//!
//! Pure functions mirroring servoshell's headed-window mapping at the pinned
//! Servo revision (`desktop/headed_window.rs`); keyboard mapping lives in
//! `keyutils`. Positions are **device pixels** on both sides: winit reports
//! physical positions and Servo takes a `DevicePoint`, converting to CSS
//! pixels itself from the `WebView`'s `HiDPI` scale factor.

use servo::{
    Cursor, DevicePoint, MouseButton, MouseButtonAction, Theme, TouchEventType, TouchId,
    WheelDelta, WheelMode,
};
use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton as WinitMouseButton, MouseScrollDelta, TouchPhase};
use winit::window::{CursorIcon, Theme as WinitTheme};

/// Device pixels scrolled per wheel line, on both axes — servoshell's
/// `LINE_HEIGHT` / `LINE_WIDTH`.
pub const WHEEL_LINE_PIXELS: f32 = 76.0;

/// Maps a winit physical position to a Servo device point.
#[expect(
    clippy::cast_possible_truncation,
    reason = "window coordinates fit an f32"
)]
pub const fn device_point_from_winit(position: PhysicalPosition<f64>) -> DevicePoint {
    DevicePoint::new(position.x as f32, position.y as f32)
}

/// Narrows winit's scale factor to the `f32` Servo's `HiDPI` scale takes.
#[expect(clippy::cast_possible_truncation, reason = "scale factors are small")]
pub const fn hidpi_scale_from_winit(scale_factor: f64) -> f32 {
    scale_factor as f32
}

/// Whether `point` is inside a viewport of `width` × `height` device pixels
/// with its origin at the window's top left: the origin edges are inside, the
/// far edges outside.
pub fn viewport_contains(width: f32, height: f32, point: DevicePoint) -> bool {
    (0.0..width).contains(&point.x) && (0.0..height).contains(&point.y)
}

/// The point inside a viewport of `width` × `height` device pixels nearest to
/// `point`: each axis is clamped to the first and last pixel. A zero-size
/// viewport clamps to the origin.
pub fn clamp_to_viewport(width: f32, height: f32, point: DevicePoint) -> DevicePoint {
    DevicePoint::new(
        point.x.clamp(0.0, (width - 1.0).max(0.0)),
        point.y.clamp(0.0, (height - 1.0).max(0.0)),
    )
}

/// Where a button event at `point` is delivered, in a viewport of `width` ×
/// `height` device pixels: a point inside is used as is; outside, a press is
/// dropped (`None`) and a release is clamped inside, so the page always sees
/// the end of a drag that left the window.
pub fn mouse_button_point(
    state: ElementState,
    width: f32,
    height: f32,
    point: DevicePoint,
) -> Option<DevicePoint> {
    if viewport_contains(width, height, point) {
        return Some(point);
    }
    match state {
        ElementState::Pressed => None,
        ElementState::Released => Some(clamp_to_viewport(width, height, point)),
    }
}

/// Maps a mouse button. Every winit button has a Servo counterpart.
pub const fn mouse_button_from_winit(button: WinitMouseButton) -> MouseButton {
    match button {
        WinitMouseButton::Left => MouseButton::Left,
        WinitMouseButton::Right => MouseButton::Right,
        WinitMouseButton::Middle => MouseButton::Middle,
        WinitMouseButton::Back => MouseButton::Back,
        WinitMouseButton::Forward => MouseButton::Forward,
        WinitMouseButton::Other(value) => MouseButton::Other(value),
    }
}

/// Maps a button's pressed/released state.
pub const fn mouse_button_action_from_winit(state: ElementState) -> MouseButtonAction {
    match state {
        ElementState::Pressed => MouseButtonAction::Down,
        ElementState::Released => MouseButtonAction::Up,
    }
}

/// Maps a scroll delta to a Servo wheel delta, always in pixel mode: line
/// deltas (a mouse wheel) are scaled by [`WHEEL_LINE_PIXELS`], pixel deltas (a
/// touchpad) pass through. Signs pass through unchanged — both sides use
/// positive for content moving right / down; Servo negates the delta itself
/// when it scrolls.
pub fn wheel_delta_from_winit(delta: MouseScrollDelta) -> WheelDelta {
    let (x, y) = match delta {
        MouseScrollDelta::LineDelta(x, y) => (
            f64::from(x * WHEEL_LINE_PIXELS),
            f64::from(y * WHEEL_LINE_PIXELS),
        ),
        MouseScrollDelta::PixelDelta(delta) => (delta.x, delta.y),
    };
    WheelDelta {
        x,
        y,
        z: 0.0,
        mode: WheelMode::DeltaPixel,
    }
}

/// Maps a touch phase.
pub const fn touch_event_type_from_winit(phase: TouchPhase) -> TouchEventType {
    match phase {
        TouchPhase::Started => TouchEventType::Down,
        TouchPhase::Moved => TouchEventType::Move,
        TouchPhase::Ended => TouchEventType::Up,
        TouchPhase::Cancelled => TouchEventType::Cancel,
    }
}

/// Maps a touch point's id. winit's ids are small per-device slot numbers, so
/// the narrowing never loses anything in practice; it wraps like servoshell's.
#[expect(
    clippy::cast_possible_truncation,
    reason = "touch ids are small slot numbers"
)]
pub const fn touch_id_from_winit(id: u64) -> TouchId {
    TouchId(id as i32)
}

/// Maps the window theme (the page's `prefers-color-scheme`).
pub const fn theme_from_winit(theme: WinitTheme) -> Theme {
    match theme {
        WinitTheme::Light => Theme::Light,
        WinitTheme::Dark => Theme::Dark,
    }
}

/// Maps the cursor a page asks for to a winit icon; `None` means the cursor is
/// hidden (`cursor: none`).
pub const fn cursor_icon_from_servo(cursor: Cursor) -> Option<CursorIcon> {
    Some(match cursor {
        Cursor::None => return None,
        Cursor::Default => CursorIcon::Default,
        Cursor::Pointer => CursorIcon::Pointer,
        Cursor::ContextMenu => CursorIcon::ContextMenu,
        Cursor::Help => CursorIcon::Help,
        Cursor::Progress => CursorIcon::Progress,
        Cursor::Wait => CursorIcon::Wait,
        Cursor::Cell => CursorIcon::Cell,
        Cursor::Crosshair => CursorIcon::Crosshair,
        Cursor::Text => CursorIcon::Text,
        Cursor::VerticalText => CursorIcon::VerticalText,
        Cursor::Alias => CursorIcon::Alias,
        Cursor::Copy => CursorIcon::Copy,
        Cursor::Move => CursorIcon::Move,
        Cursor::NoDrop => CursorIcon::NoDrop,
        Cursor::NotAllowed => CursorIcon::NotAllowed,
        Cursor::Grab => CursorIcon::Grab,
        Cursor::Grabbing => CursorIcon::Grabbing,
        Cursor::EResize => CursorIcon::EResize,
        Cursor::NResize => CursorIcon::NResize,
        Cursor::NeResize => CursorIcon::NeResize,
        Cursor::NwResize => CursorIcon::NwResize,
        Cursor::SResize => CursorIcon::SResize,
        Cursor::SeResize => CursorIcon::SeResize,
        Cursor::SwResize => CursorIcon::SwResize,
        Cursor::WResize => CursorIcon::WResize,
        Cursor::EwResize => CursorIcon::EwResize,
        Cursor::NsResize => CursorIcon::NsResize,
        Cursor::NeswResize => CursorIcon::NeswResize,
        Cursor::NwseResize => CursorIcon::NwseResize,
        Cursor::ColResize => CursorIcon::ColResize,
        Cursor::RowResize => CursorIcon::RowResize,
        Cursor::AllScroll => CursorIcon::AllScroll,
        Cursor::ZoomIn => CursorIcon::ZoomIn,
        Cursor::ZoomOut => CursorIcon::ZoomOut,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_point_keeps_physical_pixels() {
        let point = device_point_from_winit(PhysicalPosition::new(12.5, 300.25));
        assert_eq!(point, DevicePoint::new(12.5, 300.25));
    }

    #[test]
    fn viewport_contains_origin_but_not_far_edges() {
        assert!(viewport_contains(800.0, 600.0, DevicePoint::new(0.0, 0.0)));
        assert!(viewport_contains(
            800.0,
            600.0,
            DevicePoint::new(799.5, 599.5)
        ));
        assert!(!viewport_contains(
            800.0,
            600.0,
            DevicePoint::new(800.0, 10.0)
        ));
        assert!(!viewport_contains(
            800.0,
            600.0,
            DevicePoint::new(10.0, 600.0)
        ));
        assert!(!viewport_contains(
            800.0,
            600.0,
            DevicePoint::new(-1.0, 10.0)
        ));
        assert!(!viewport_contains(
            800.0,
            600.0,
            DevicePoint::new(10.0, -1.0)
        ));
    }

    #[test]
    fn clamp_pulls_outside_points_to_the_nearest_pixel() {
        let clamped = clamp_to_viewport(800.0, 600.0, DevicePoint::new(-5.0, 700.0));
        assert_eq!(clamped, DevicePoint::new(0.0, 599.0));
        let clamped = clamp_to_viewport(800.0, 600.0, DevicePoint::new(900.0, -1.0));
        assert_eq!(clamped, DevicePoint::new(799.0, 0.0));
        assert!(viewport_contains(800.0, 600.0, clamped));
        // the last fractional pixel is inside the viewport but still clamps
        let clamped = clamp_to_viewport(800.0, 600.0, DevicePoint::new(799.5, 10.0));
        assert_eq!(clamped, DevicePoint::new(799.0, 10.0));
    }

    #[test]
    fn button_point_inside_is_identity() {
        let point = DevicePoint::new(799.5, 10.0);
        for state in [ElementState::Pressed, ElementState::Released] {
            assert_eq!(mouse_button_point(state, 800.0, 600.0, point), Some(point));
        }
    }

    #[test]
    fn button_press_outside_is_dropped() {
        let point = DevicePoint::new(-5.0, 700.0);
        assert_eq!(
            mouse_button_point(ElementState::Pressed, 800.0, 600.0, point),
            None
        );
    }

    #[test]
    fn button_release_outside_is_clamped() {
        let point = DevicePoint::new(-5.0, 700.0);
        assert_eq!(
            mouse_button_point(ElementState::Released, 800.0, 600.0, point),
            Some(DevicePoint::new(0.0, 599.0))
        );
        // exactly on the far edge is outside (the viewport is half-open)
        assert_eq!(
            mouse_button_point(
                ElementState::Released,
                800.0,
                600.0,
                DevicePoint::new(800.0, 10.0)
            ),
            Some(DevicePoint::new(799.0, 10.0))
        );
        assert_eq!(
            mouse_button_point(
                ElementState::Pressed,
                800.0,
                600.0,
                DevicePoint::new(800.0, 10.0)
            ),
            None
        );
    }

    #[test]
    fn clamp_keeps_inside_points() {
        let point = DevicePoint::new(12.5, 300.25);
        assert_eq!(clamp_to_viewport(800.0, 600.0, point), point);
    }

    #[test]
    fn clamp_survives_a_zero_size_viewport() {
        let clamped = clamp_to_viewport(0.0, 0.0, DevicePoint::new(40.0, -3.0));
        assert_eq!(clamped, DevicePoint::new(0.0, 0.0));
    }

    #[test]
    fn mouse_buttons_map_by_name() {
        let cases = [
            (WinitMouseButton::Left, MouseButton::Left),
            (WinitMouseButton::Right, MouseButton::Right),
            (WinitMouseButton::Middle, MouseButton::Middle),
            (WinitMouseButton::Back, MouseButton::Back),
            (WinitMouseButton::Forward, MouseButton::Forward),
            (WinitMouseButton::Other(9), MouseButton::Other(9)),
        ];
        for (winit_button, expected) in cases {
            assert_eq!(mouse_button_from_winit(winit_button), expected);
        }
    }

    #[test]
    fn mouse_button_actions_map() {
        assert_eq!(
            mouse_button_action_from_winit(ElementState::Pressed),
            MouseButtonAction::Down
        );
        assert_eq!(
            mouse_button_action_from_winit(ElementState::Released),
            MouseButtonAction::Up
        );
    }

    #[test]
    fn wheel_line_delta_scales_to_pixels_keeping_sign() {
        let delta = wheel_delta_from_winit(MouseScrollDelta::LineDelta(1.0, -2.0));
        assert_eq!(
            delta,
            WheelDelta {
                x: 76.0,
                y: -152.0,
                z: 0.0,
                mode: WheelMode::DeltaPixel
            }
        );
    }

    #[test]
    fn wheel_pixel_delta_passes_through() {
        let delta = wheel_delta_from_winit(MouseScrollDelta::PixelDelta(PhysicalPosition::new(
            -3.5, 12.25,
        )));
        assert_eq!(
            delta,
            WheelDelta {
                x: -3.5,
                y: 12.25,
                z: 0.0,
                mode: WheelMode::DeltaPixel
            }
        );
    }

    #[test]
    fn touch_phases_map() {
        let cases = [
            (TouchPhase::Started, TouchEventType::Down),
            (TouchPhase::Moved, TouchEventType::Move),
            (TouchPhase::Ended, TouchEventType::Up),
            (TouchPhase::Cancelled, TouchEventType::Cancel),
        ];
        for (phase, expected) in cases {
            // `TouchEventType` has no `PartialEq`; compare discriminants
            assert_eq!(
                std::mem::discriminant(&touch_event_type_from_winit(phase)),
                std::mem::discriminant(&expected)
            );
        }
    }

    #[test]
    fn touch_ids_keep_small_values() {
        assert_eq!(touch_id_from_winit(0), TouchId(0));
        assert_eq!(touch_id_from_winit(7), TouchId(7));
    }

    #[test]
    fn themes_map() {
        assert_eq!(theme_from_winit(WinitTheme::Light), Theme::Light);
        assert_eq!(theme_from_winit(WinitTheme::Dark), Theme::Dark);
    }

    #[test]
    fn cursor_none_hides_and_others_map_by_name() {
        assert_eq!(cursor_icon_from_servo(Cursor::None), None);
        assert_eq!(
            cursor_icon_from_servo(Cursor::Default),
            Some(CursorIcon::Default)
        );
        assert_eq!(
            cursor_icon_from_servo(Cursor::Pointer),
            Some(CursorIcon::Pointer)
        );
        assert_eq!(cursor_icon_from_servo(Cursor::Text), Some(CursorIcon::Text));
        assert_eq!(
            cursor_icon_from_servo(Cursor::NwseResize),
            Some(CursorIcon::NwseResize)
        );
    }
}
