//! Reading controllers. `gilrs` (evdev/XInput) wants polling and isn't thread-safe, so it
//! runs on a dedicated OS thread turning device state into events.
//!
//! - **Buttons** are edge triggered, with direction auto-repeat applied here (a busy
//!   webview cannot keep a steady cadence).
//! - **Axes** are sampled and sent only outside the dead zone, for scrolling.
//!
//! Nothing here decides what a button *does*; the frontend owns that.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// How often the pad is sampled. 60Hz: fast enough that a press never feels dropped.
const POLL_INTERVAL: Duration = Duration::from_millis(16);

/// How long a direction must be held before it starts repeating.
///
/// Long enough that a single deliberate press moves exactly one item, which is the whole
/// difficulty with stick navigation.
const REPEAT_DELAY: Duration = Duration::from_millis(400);

/// The cadence once repeating has started.
const REPEAT_INTERVAL: Duration = Duration::from_millis(140);

/// Past this, a trigger counts as pressed.
const TRIGGER_PRESS: f32 = 0.65;

/// Below this, it counts as released. The gap stops a half-held trigger chattering.
const TRIGGER_RELEASE: f32 = 0.45;

/// A logical button, named for what the frontend binds rather than for a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Button {
    South,
    East,
    West,
    North,
    LeftBumper,
    RightBumper,
    LeftTrigger,
    RightTrigger,
    Select,
    Start,
    Up,
    Down,
    Left,
    Right,
}

impl Button {
    /// Directions auto-repeat when held; face buttons must not.
    fn repeats(self) -> bool {
        matches!(
            self,
            Button::Up | Button::Down | Button::Left | Button::Right
        )
    }
}

/// One button going down, sent to the webview.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ButtonEvent {
    button: Button,
    /// True when this is an auto-repeat rather than a fresh press, so the frontend can
    /// choose to ignore repeats where they make no sense, such as activating a button.
    repeat: bool,
}

/// Stick position, for continuous scrolling.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
struct AxisEvent {
    right_x: f32,
    right_y: f32,
}

/// Whether a controller is attached at all.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectionEvent {
    connected: bool,
    /// The pad's name, for showing which one is in use.
    name: Option<String>,
}

/// Lets the poll thread be told to stop reading without being torn down.
#[derive(Clone, Default)]
pub struct Handle {
    enabled: Arc<AtomicBool>,
    deadzone: Arc<std::sync::RwLock<f32>>,
}

impl Handle {
    pub fn new(enabled: bool, deadzone: f64) -> Self {
        Self {
            enabled: Arc::new(AtomicBool::new(enabled)),
            deadzone: Arc::new(std::sync::RwLock::new(deadzone as f32)),
        }
    }

    /// Turn reading on or off. Takes effect on the next poll.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    pub fn set_deadzone(&self, deadzone: f64) {
        if let Ok(mut slot) = self.deadzone.write() {
            *slot = deadzone.clamp(0.05, 0.9) as f32;
        }
    }

    fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    fn deadzone(&self) -> f32 {
        self.deadzone.read().map(|d| *d).unwrap_or(0.25)
    }
}

/// Translate a `gilrs` button into ours, ignoring the ones nothing is bound to.
fn map_button(button: gilrs::Button) -> Option<Button> {
    use gilrs::Button as G;
    Some(match button {
        G::South => Button::South,
        G::East => Button::East,
        G::West => Button::West,
        G::North => Button::North,
        G::LeftTrigger => Button::LeftBumper,
        G::RightTrigger => Button::RightBumper,
        G::LeftTrigger2 => Button::LeftTrigger,
        G::RightTrigger2 => Button::RightTrigger,
        G::Select => Button::Select,
        G::Start => Button::Start,
        G::DPadUp => Button::Up,
        G::DPadDown => Button::Down,
        G::DPadLeft => Button::Left,
        G::DPadRight => Button::Right,
        _ => return None,
    })
}

/// Which direction a stick position represents, if any.
///
/// The larger axis wins, so a diagonal push resolves to one direction rather than firing
/// both and moving the focus two places at once.
fn stick_direction(x: f32, y: f32, deadzone: f32) -> Option<Button> {
    if x.abs() < deadzone && y.abs() < deadzone {
        return None;
    }
    if x.abs() > y.abs() {
        Some(if x > 0.0 { Button::Right } else { Button::Left })
    } else {
        // gilrs reports the stick's Y as positive when pushed up.
        Some(if y > 0.0 { Button::Up } else { Button::Down })
    }
}

/// Tracks how long a direction has been held, and when it should fire again.
struct Repeater {
    held: Option<Button>,
    since: Instant,
    last_fired: Instant,
}

impl Repeater {
    fn new() -> Self {
        Self {
            held: None,
            since: Instant::now(),
            last_fired: Instant::now(),
        }
    }

    /// Report the direction currently held, returning one if it should fire now.
    fn update(&mut self, direction: Option<Button>) -> Option<(Button, bool)> {
        let now = Instant::now();
        match direction {
            None => {
                self.held = None;
                None
            }
            Some(button) if self.held != Some(button) => {
                // A new direction fires at once, then waits out the delay.
                self.held = Some(button);
                self.since = now;
                self.last_fired = now;
                Some((button, false))
            }
            Some(button) => {
                if now.duration_since(self.since) < REPEAT_DELAY {
                    return None;
                }
                if now.duration_since(self.last_fired) < REPEAT_INTERVAL {
                    return None;
                }
                self.last_fired = now;
                Some((button, true))
            }
        }
    }
}

/// Tracks a trigger's pressed state with hysteresis.
#[derive(Default)]
struct TriggerState {
    pressed: bool,
}

impl TriggerState {
    /// True when the trigger has just crossed into being pressed.
    fn update(&mut self, value: f32) -> bool {
        if self.pressed {
            if value < TRIGGER_RELEASE {
                self.pressed = false;
            }
            false
        } else if value > TRIGGER_PRESS {
            self.pressed = true;
            true
        } else {
            false
        }
    }
}

/// Start the polling thread.
///
/// A plain OS thread rather than a task: `gilrs` is not `Send`, so it has to be created
/// and used on the same thread, and it wants to be polled rather than awaited.
pub fn spawn(app: AppHandle, handle: Handle) {
    std::thread::Builder::new()
        .name("gamepad".into())
        .spawn(move || run(app, handle))
        .map(|_| ())
        .unwrap_or_else(|e| tracing::warn!("could not start the gamepad thread: {e}"));
}

fn run(app: AppHandle, handle: Handle) {
    let mut gilrs = match gilrs::Gilrs::new() {
        Ok(gilrs) => gilrs,
        Err(e) => {
            // Entirely survivable: a machine with no input devices, or a sandbox without
            // permission to read them. The app works from the keyboard and mouse.
            tracing::info!("controller support unavailable: {e}");
            return;
        }
    };

    tracing::info!("gamepad polling started");
    let mut repeater = Repeater::new();
    let mut left_trigger = TriggerState::default();
    let mut right_trigger = TriggerState::default();
    let mut connected = false;

    loop {
        std::thread::sleep(POLL_INTERVAL);

        // Events must be drained even while disabled, or the queue grows without bound
        // and re-enabling replays every press made in between.
        let mut pressed = Vec::new();
        while let Some(event) = gilrs.next_event() {
            if let gilrs::EventType::ButtonPressed(button, _) = event.event {
                if let Some(mapped) = map_button(button) {
                    // Directions are driven by the held-state scan below so the d-pad and
                    // the stick repeat identically; a raw press here would double them up.
                    if !mapped.repeats() {
                        pressed.push(mapped);
                    }
                }
            }
        }

        if !handle.enabled() {
            if connected {
                connected = false;
                let _ = app.emit(
                    "gamepad-connection",
                    ConnectionEvent {
                        connected: false,
                        name: None,
                    },
                );
            }
            continue;
        }

        // The first connected pad wins. Supporting several would mean deciding what two
        // people pressing different directions means, which is not a couch-launcher
        // problem worth solving.
        let active = gilrs.gamepads().find(|(_, pad)| pad.is_connected());

        let Some((_, pad)) = active else {
            if connected {
                connected = false;
                tracing::info!("controller disconnected");
                let _ = app.emit(
                    "gamepad-connection",
                    ConnectionEvent {
                        connected: false,
                        name: None,
                    },
                );
            }
            continue;
        };

        if !connected {
            connected = true;
            tracing::info!(name = pad.name(), "controller connected");
            let _ = app.emit(
                "gamepad-connection",
                ConnectionEvent {
                    connected: true,
                    name: Some(pad.name().to_string()),
                },
            );
        }

        for button in pressed {
            let _ = app.emit(
                "gamepad-button",
                ButtonEvent {
                    button,
                    repeat: false,
                },
            );
        }

        let deadzone = handle.deadzone();

        // Triggers are axes on most pads and buttons on some, so both are read and the
        // hysteresis state settles which it was.
        let lt =
            axis(&pad, gilrs::Axis::LeftZ).max(button_value(&pad, gilrs::Button::LeftTrigger2));
        let rt =
            axis(&pad, gilrs::Axis::RightZ).max(button_value(&pad, gilrs::Button::RightTrigger2));
        if left_trigger.update(lt) {
            let _ = app.emit(
                "gamepad-button",
                ButtonEvent {
                    button: Button::LeftTrigger,
                    repeat: false,
                },
            );
        }
        if right_trigger.update(rt) {
            let _ = app.emit(
                "gamepad-button",
                ButtonEvent {
                    button: Button::RightTrigger,
                    repeat: false,
                },
            );
        }

        // The d-pad and the left stick are one input as far as navigation is concerned.
        let dpad = held_direction(&pad);
        let stick = stick_direction(
            axis(&pad, gilrs::Axis::LeftStickX),
            axis(&pad, gilrs::Axis::LeftStickY),
            deadzone,
        );
        if let Some((button, repeat)) = repeater.update(dpad.or(stick)) {
            let _ = app.emit("gamepad-button", ButtonEvent { button, repeat });
        }

        // The right stick scrolls, and is sent as a position rather than as steps.
        let right_x = axis(&pad, gilrs::Axis::RightStickX);
        let right_y = axis(&pad, gilrs::Axis::RightStickY);
        if right_x.abs() >= deadzone || right_y.abs() >= deadzone {
            let _ = app.emit("gamepad-axis", AxisEvent { right_x, right_y });
        }
    }
}

fn axis(pad: &gilrs::Gamepad<'_>, axis: gilrs::Axis) -> f32 {
    pad.axis_data(axis).map(|data| data.value()).unwrap_or(0.0)
}

fn button_value(pad: &gilrs::Gamepad<'_>, button: gilrs::Button) -> f32 {
    pad.button_data(button)
        .map(|data| data.value())
        .unwrap_or(0.0)
}

/// Which d-pad direction is held, if any.
fn held_direction(pad: &gilrs::Gamepad<'_>) -> Option<Button> {
    for (button, direction) in [
        (gilrs::Button::DPadUp, Button::Up),
        (gilrs::Button::DPadDown, Button::Down),
        (gilrs::Button::DPadLeft, Button::Left),
        (gilrs::Button::DPadRight, Button::Right),
    ] {
        if pad.is_pressed(button) {
            return Some(direction);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_directions_repeat() {
        // Auto-repeating "activate" would launch a game several times from one press.
        assert!(Button::Up.repeats());
        assert!(Button::Left.repeats());
        assert!(!Button::South.repeats());
        assert!(!Button::Start.repeats());
    }

    #[test]
    fn a_stick_inside_the_dead_zone_is_not_a_direction() {
        // A worn stick rests off centre; without this it reads as a held direction
        // forever and the focus never stops moving.
        assert_eq!(stick_direction(0.1, 0.1, 0.25), None);
        assert_eq!(stick_direction(0.0, 0.0, 0.25), None);
    }

    #[test]
    fn a_diagonal_push_resolves_to_one_direction() {
        // Firing both would move the focus two places from one gesture.
        assert_eq!(stick_direction(0.9, 0.4, 0.25), Some(Button::Right));
        assert_eq!(stick_direction(0.4, 0.9, 0.25), Some(Button::Up));
        assert_eq!(stick_direction(-0.9, 0.4, 0.25), Some(Button::Left));
        assert_eq!(stick_direction(0.4, -0.9, 0.25), Some(Button::Down));
    }

    #[test]
    fn a_larger_dead_zone_takes_more_push() {
        assert_eq!(stick_direction(0.3, 0.0, 0.25), Some(Button::Right));
        assert_eq!(stick_direction(0.3, 0.0, 0.5), None);
    }

    #[test]
    fn a_trigger_needs_a_full_release_before_it_fires_again() {
        // Without the gap, a trigger held near the threshold chatters.
        let mut trigger = TriggerState::default();
        assert!(!trigger.update(0.5), "below the press threshold");
        assert!(trigger.update(0.7), "crossed it");
        assert!(!trigger.update(0.9), "already pressed, not a new press");
        assert!(!trigger.update(0.5), "still above the release threshold");
        assert!(!trigger.update(0.2), "released, but that is not a press");
        assert!(trigger.update(0.7), "pressed again");
    }

    #[test]
    fn a_new_direction_fires_immediately() {
        let mut repeater = Repeater::new();
        assert_eq!(
            repeater.update(Some(Button::Down)),
            Some((Button::Down, false))
        );
        // Held, but not yet long enough to repeat.
        assert_eq!(repeater.update(Some(Button::Down)), None);
        // Changing direction fires at once rather than waiting out the old delay.
        assert_eq!(repeater.update(Some(Button::Up)), Some((Button::Up, false)));
    }

    #[test]
    fn releasing_clears_the_repeat() {
        let mut repeater = Repeater::new();
        repeater.update(Some(Button::Down));
        assert_eq!(repeater.update(None), None);
        // The same direction after a release is a fresh press, not a repeat.
        assert_eq!(
            repeater.update(Some(Button::Down)),
            Some((Button::Down, false))
        );
    }

    #[test]
    fn a_held_direction_repeats_once_the_delay_has_passed() {
        let mut repeater = Repeater::new();
        repeater.update(Some(Button::Down));
        // Rather than sleeping, wind the clock back past both thresholds.
        repeater.since -= REPEAT_DELAY + Duration::from_millis(10);
        repeater.last_fired -= REPEAT_INTERVAL + Duration::from_millis(10);
        assert_eq!(
            repeater.update(Some(Button::Down)),
            Some((Button::Down, true))
        );
        // And not again until the interval has passed once more.
        assert_eq!(repeater.update(Some(Button::Down)), None);
    }

    #[test]
    fn the_dead_zone_is_clamped_to_something_usable() {
        // A zero dead zone makes every pad drift; a huge one makes the stick feel dead.
        let handle = Handle::new(true, 0.25);
        handle.set_deadzone(0.0);
        assert!(handle.deadzone() >= 0.05);
        handle.set_deadzone(5.0);
        assert!(handle.deadzone() <= 0.9);
    }

    #[test]
    fn reading_can_be_turned_off_and_on() {
        let handle = Handle::new(true, 0.25);
        assert!(handle.enabled());
        handle.set_enabled(false);
        assert!(!handle.enabled());
        handle.set_enabled(true);
        assert!(handle.enabled());
    }

    #[test]
    fn the_face_buttons_map_to_their_logical_names() {
        assert_eq!(map_button(gilrs::Button::South), Some(Button::South));
        assert_eq!(map_button(gilrs::Button::DPadLeft), Some(Button::Left));
        // Shoulder and trigger naming in gilrs is off by one from what people say aloud:
        // `LeftTrigger` is the bumper and `LeftTrigger2` the analogue trigger.
        assert_eq!(
            map_button(gilrs::Button::LeftTrigger),
            Some(Button::LeftBumper)
        );
        assert_eq!(
            map_button(gilrs::Button::LeftTrigger2),
            Some(Button::LeftTrigger)
        );
        assert_eq!(map_button(gilrs::Button::Unknown), None);
    }
}
