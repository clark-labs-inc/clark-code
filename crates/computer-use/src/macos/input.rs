mod monitor;

use std::time::Duration;

use core_graphics::event::{
    CGEvent, CGEventFlags, CGEventTapLocation, CGEventType, KeyCode, ScrollEventUnit,
};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGPoint;

use crate::lease::InputLease;
use crate::{ComputerUseError, Key, Modifier, MouseButton, Point};

pub use monitor::PhysicalInputMonitor;

const SETTLE: Duration = Duration::from_millis(12);
const EVENT_TAG: i64 = 0x434c_4152_4b43_5531;

pub fn click(
    point: Point,
    button: MouseButton,
    lease: &InputLease,
) -> Result<(), ComputerUseError> {
    let source = source()?;
    let (down_type, up_type, cg_button) = button_events(button);
    let position = CGPoint::new(point.x, point.y);
    let down = CGEvent::new_mouse_event(source.clone(), down_type, position, cg_button)
        .map_err(|_| ComputerUseError::Os("could not create mouse-down event".to_string()))?;
    let up = CGEvent::new_mouse_event(source, up_type, position, cg_button)
        .map_err(|_| ComputerUseError::Os("could not create mouse-up event".to_string()))?;
    post(&down, lease)?;
    std::thread::sleep(SETTLE);
    if let Err(error) = lease.check() {
        post_cleanup(&up);
        return Err(error);
    }
    post(&up, lease)?;
    std::thread::sleep(SETTLE);
    Ok(())
}

pub fn drag(
    start: Point,
    end: Point,
    button: MouseButton,
    duration_ms: u32,
    lease: &InputLease,
) -> Result<(), ComputerUseError> {
    let source = source()?;
    let (down_type, up_type, cg_button) = button_events(button);
    let drag_type = match button {
        MouseButton::Left => CGEventType::LeftMouseDragged,
        MouseButton::Right => CGEventType::RightMouseDragged,
    };
    let down = CGEvent::new_mouse_event(
        source.clone(),
        down_type,
        CGPoint::new(start.x, start.y),
        cg_button,
    )
    .map_err(|_| ComputerUseError::Os("could not create drag mouse-down event".to_string()))?;
    post(&down, lease)?;
    let steps = (duration_ms / 16).clamp(2, 120);
    let delay = Duration::from_millis((duration_ms as u64 / steps as u64).max(1));
    for step in 1..=steps {
        let current = Point {
            x: start.x + (end.x - start.x) * step as f64 / steps as f64,
            y: start.y + (end.y - start.y) * step as f64 / steps as f64,
        };
        if let Err(error) = lease.check() {
            post_drag_cleanup(&source, up_type, current, cg_button);
            return Err(error);
        }
        let event = CGEvent::new_mouse_event(
            source.clone(),
            drag_type,
            CGPoint::new(current.x, current.y),
            cg_button,
        )
        .map_err(|_| ComputerUseError::Os("could not create mouse-drag event".to_string()))?;
        post(&event, lease)?;
        std::thread::sleep(delay);
    }
    let up = CGEvent::new_mouse_event(source, up_type, CGPoint::new(end.x, end.y), cg_button)
        .map_err(|_| ComputerUseError::Os("could not create drag mouse-up event".to_string()))?;
    if let Err(error) = lease.check() {
        post_cleanup(&up);
        return Err(error);
    }
    post(&up, lease)?;
    Ok(())
}

pub fn scroll(
    point: Point,
    delta_x: i32,
    delta_y: i32,
    lease: &InputLease,
) -> Result<(), ComputerUseError> {
    let source = source()?;
    let move_event = CGEvent::new_mouse_event(
        source.clone(),
        CGEventType::MouseMoved,
        CGPoint::new(point.x, point.y),
        core_graphics::event::CGMouseButton::Left,
    )
    .map_err(|_| ComputerUseError::Os("could not create scroll positioning event".to_string()))?;
    post(&move_event, lease)?;
    let event = CGEvent::new_scroll_event(source, ScrollEventUnit::PIXEL, 2, delta_y, delta_x, 0)
        .map_err(|_| ComputerUseError::Os("could not create scroll event".to_string()))?;
    post(&event, lease)?;
    std::thread::sleep(SETTLE);
    Ok(())
}

pub fn type_text(text: &str, lease: &InputLease) -> Result<(), ComputerUseError> {
    let source = source()?;
    for chunk in unicode_chunks(text, 20) {
        let event = CGEvent::new_keyboard_event(source.clone(), 0, true)
            .map_err(|_| ComputerUseError::Os("could not create text event".to_string()))?;
        event.set_string(chunk);
        post(&event, lease)?;
        std::thread::sleep(SETTLE);
    }
    Ok(())
}

pub fn keypress(
    key: Key,
    modifiers: &[Modifier],
    lease: &InputLease,
) -> Result<(), ComputerUseError> {
    let keycode = keycode(key)?;
    let source = source()?;
    let down = CGEvent::new_keyboard_event(source.clone(), keycode, true)
        .map_err(|_| ComputerUseError::Os("could not create key-down event".to_string()))?;
    let up = CGEvent::new_keyboard_event(source, keycode, false)
        .map_err(|_| ComputerUseError::Os("could not create key-up event".to_string()))?;
    let flags = modifier_flags(modifiers);
    down.set_flags(flags);
    up.set_flags(flags);
    post(&down, lease)?;
    std::thread::sleep(SETTLE);
    if let Err(error) = lease.check() {
        post_cleanup(&up);
        return Err(error);
    }
    post(&up, lease)?;
    std::thread::sleep(SETTLE);
    Ok(())
}

fn post(event: &CGEvent, lease: &InputLease) -> Result<(), ComputerUseError> {
    lease.check()?;
    event.set_integer_value_field(
        core_graphics::event::EventField::EVENT_SOURCE_USER_DATA,
        EVENT_TAG,
    );
    event.post(CGEventTapLocation::HID);
    Ok(())
}

fn post_cleanup(event: &CGEvent) {
    event.set_integer_value_field(
        core_graphics::event::EventField::EVENT_SOURCE_USER_DATA,
        EVENT_TAG,
    );
    event.post(CGEventTapLocation::HID);
}

fn post_drag_cleanup(
    source: &CGEventSource,
    up_type: CGEventType,
    point: Point,
    button: core_graphics::event::CGMouseButton,
) {
    if let Ok(up) = CGEvent::new_mouse_event(
        source.clone(),
        up_type,
        CGPoint::new(point.x, point.y),
        button,
    ) {
        post_cleanup(&up);
    }
}

fn button_events(
    button: MouseButton,
) -> (
    CGEventType,
    CGEventType,
    core_graphics::event::CGMouseButton,
) {
    match button {
        MouseButton::Left => (
            CGEventType::LeftMouseDown,
            CGEventType::LeftMouseUp,
            core_graphics::event::CGMouseButton::Left,
        ),
        MouseButton::Right => (
            CGEventType::RightMouseDown,
            CGEventType::RightMouseUp,
            core_graphics::event::CGMouseButton::Right,
        ),
    }
}

fn source() -> Result<CGEventSource, ComputerUseError> {
    CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|_| ComputerUseError::Os("could not create CGEvent source".to_string()))
}

fn modifier_flags(modifiers: &[Modifier]) -> CGEventFlags {
    modifiers
        .iter()
        .fold(CGEventFlags::CGEventFlagNull, |flags, modifier| {
            flags
                | match modifier {
                    Modifier::Command => CGEventFlags::CGEventFlagCommand,
                    Modifier::Control => CGEventFlags::CGEventFlagControl,
                    Modifier::Option => CGEventFlags::CGEventFlagAlternate,
                    Modifier::Shift => CGEventFlags::CGEventFlagShift,
                }
        })
}

fn keycode(key: Key) -> Result<u16, ComputerUseError> {
    Ok(match key {
        Key::Return => KeyCode::RETURN,
        Key::Escape => KeyCode::ESCAPE,
        Key::Tab => KeyCode::TAB,
        Key::Space => KeyCode::SPACE,
        Key::Backspace => KeyCode::DELETE,
        Key::Delete => KeyCode::FORWARD_DELETE,
        Key::ArrowUp => KeyCode::UP_ARROW,
        Key::ArrowDown => KeyCode::DOWN_ARROW,
        Key::ArrowLeft => KeyCode::LEFT_ARROW,
        Key::ArrowRight => KeyCode::RIGHT_ARROW,
        Key::Home => KeyCode::HOME,
        Key::End => KeyCode::END,
        Key::PageUp => KeyCode::PAGE_UP,
        Key::PageDown => KeyCode::PAGE_DOWN,
        Key::Character(character) => character_keycode(character)?,
    })
}

fn character_keycode(character: char) -> Result<u16, ComputerUseError> {
    let keycode = match character.to_ascii_lowercase() {
        'a' => KeyCode::ANSI_A,
        'b' => KeyCode::ANSI_B,
        'c' => KeyCode::ANSI_C,
        'd' => KeyCode::ANSI_D,
        'e' => KeyCode::ANSI_E,
        'f' => KeyCode::ANSI_F,
        'g' => KeyCode::ANSI_G,
        'h' => KeyCode::ANSI_H,
        'i' => KeyCode::ANSI_I,
        'j' => KeyCode::ANSI_J,
        'k' => KeyCode::ANSI_K,
        'l' => KeyCode::ANSI_L,
        'm' => KeyCode::ANSI_M,
        'n' => KeyCode::ANSI_N,
        'o' => KeyCode::ANSI_O,
        'p' => KeyCode::ANSI_P,
        'q' => KeyCode::ANSI_Q,
        'r' => KeyCode::ANSI_R,
        's' => KeyCode::ANSI_S,
        't' => KeyCode::ANSI_T,
        'u' => KeyCode::ANSI_U,
        'v' => KeyCode::ANSI_V,
        'w' => KeyCode::ANSI_W,
        'x' => KeyCode::ANSI_X,
        'y' => KeyCode::ANSI_Y,
        'z' => KeyCode::ANSI_Z,
        '0' => KeyCode::ANSI_0,
        '1' => KeyCode::ANSI_1,
        '2' => KeyCode::ANSI_2,
        '3' => KeyCode::ANSI_3,
        '4' => KeyCode::ANSI_4,
        '5' => KeyCode::ANSI_5,
        '6' => KeyCode::ANSI_6,
        '7' => KeyCode::ANSI_7,
        '8' => KeyCode::ANSI_8,
        '9' => KeyCode::ANSI_9,
        '=' => KeyCode::ANSI_EQUAL,
        '-' => KeyCode::ANSI_MINUS,
        ',' => KeyCode::ANSI_COMMA,
        '.' => KeyCode::ANSI_PERIOD,
        '/' => KeyCode::ANSI_SLASH,
        other => {
            return Err(ComputerUseError::Os(format!(
                "keypress does not support {other:?}; use computer_type_text for Unicode"
            )))
        }
    };
    Ok(keycode)
}

fn unicode_chunks(text: &str, max_chars: usize) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut count = 0;
    for (index, _) in text.char_indices() {
        if count == max_chars {
            chunks.push(&text[start..index]);
            start = index;
            count = 0;
        }
        count += 1;
    }
    chunks.push(&text[start..]);
    chunks
}

#[cfg(test)]
mod tests {
    use super::unicode_chunks;

    #[test]
    fn unicode_chunks_preserve_text_and_character_boundaries() {
        let text = "12345678901234567890🙂érest";
        let chunks = unicode_chunks(text, 20);

        assert_eq!(chunks, ["12345678901234567890", "🙂érest"]);
        assert_eq!(chunks.concat(), text);
    }
}
