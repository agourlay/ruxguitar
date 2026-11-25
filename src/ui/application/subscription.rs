//! The event sources the application listens to: keyboard, playback ticks,
//! window events and the end of a selection drag.

use iced::{Subscription, event, keyboard, mouse, stream, window};

use iced::futures::{SinkExt, Stream};
use iced::keyboard::key::Named::{ArrowDown, ArrowLeft, ArrowRight, ArrowUp, F11, Space};
use iced::keyboard::key::{Code, Physical};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::Notify;

use super::app::{Message, RuxApplication};

impl RuxApplication {
    #[allow(clippy::unused_self)]
    fn audio_player_beat_subscription(
        current_tick: Arc<AtomicU32>,
        beat_notify: Arc<Notify>,
    ) -> impl Stream<Item = Message> {
        stream::channel(1, async move |mut output| {
            loop {
                beat_notify.notified().await;
                let tick = current_tick.load(Ordering::Acquire);
                output
                    .send(Message::FocusTick(tick))
                    .await
                    .expect("send failed");
            }
        })
    }

    pub(super) fn subscription(&self) -> Subscription<Message> {
        let mut subscriptions = Vec::with_capacity(4);

        // keyboard event subscription
        let keyboard_subscription = keyboard::listen().filter_map(|event| {
            let keyboard::Event::KeyPressed {
                modified_key,
                physical_key,
                modifiers,
                ..
            } = event
            else {
                return None;
            };
            // by position: the digits need Shift on some layouts; not with a
            // modifier, which desktops switch workspaces and windows with
            if !modifiers.control()
                && !modifiers.alt()
                && !modifiers.logo()
                && let Some(track) = track_of_key(physical_key, modified_key.as_ref())
            {
                return Some(Message::TrackOverviewTrack(track));
            }
            match modified_key.as_ref() {
                keyboard::Key::Named(Space) => Some(Message::PlayPause),
                keyboard::Key::Named(ArrowUp) if modifiers.control() => {
                    Some(Message::IncreaseTempo)
                }
                keyboard::Key::Named(ArrowDown) if modifiers.control() => {
                    Some(Message::DecreaseTempo)
                }
                keyboard::Key::Named(ArrowLeft) => Some(Message::PreviousMeasure),
                keyboard::Key::Named(ArrowRight) => Some(Message::NextMeasure),
                keyboard::Key::Named(ArrowUp) => Some(Message::PreviousTrack),
                keyboard::Key::Named(ArrowDown) => Some(Message::NextTrack),
                keyboard::Key::Character(c) if c.eq_ignore_ascii_case("s") => {
                    Some(Message::ToggleSolo)
                }
                keyboard::Key::Character(c) if c.eq_ignore_ascii_case("m") => {
                    Some(Message::ToggleMute)
                }
                keyboard::Key::Named(F11) => Some(Message::ToggleFullscreen),
                _ => None,
            }
        });
        subscriptions.push(keyboard_subscription);

        // next beat notifier subscription
        subscriptions.push(Subscription::run_with(
            BeatSubscriptionData(self.current_tick.clone(), self.beat_notify.clone()),
            |data| Self::audio_player_beat_subscription(data.0.clone(), data.1.clone()),
        ));

        let window_resized = window::resize_events().map(|(_, size)| Message::WindowSized(size));
        subscriptions.push(window_resized);

        subscriptions.push(iced::system::theme_changes().map(Message::DesktopToneChanged));

        let file_dropped = window::events().filter_map(|(_, event)| {
            if let window::Event::FileDropped(path) = event {
                Some(Message::OpenFile(path))
            } else {
                None
            }
        });
        subscriptions.push(file_dropped);

        // closing is handled here so the session is saved before exiting
        subscriptions.push(window::close_requests().map(|_| Message::CloseRequested));

        if self.selection_drag.is_active() {
            let drag_release = event::listen_with(|event, _status, _id| {
                if matches!(
                    event,
                    iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                ) {
                    Some(Message::SelectionDragEnd)
                } else {
                    None
                }
            });
            subscriptions.push(drag_release);
        }

        Subscription::batch(subscriptions)
    }
}

/// The track a number key picks: `1` to `9` for the first nine.
///
/// The digit row counts by position, as its digits need Shift on some
/// layouts. Any other key counts when it types a digit: the numeric pad
/// does with NumLock on, and moves like the arrows with it off.
fn track_of_key(physical: Physical, typed: keyboard::Key<&str>) -> Option<usize> {
    let row = match physical {
        Physical::Code(Code::Digit1) => Some(0),
        Physical::Code(Code::Digit2) => Some(1),
        Physical::Code(Code::Digit3) => Some(2),
        Physical::Code(Code::Digit4) => Some(3),
        Physical::Code(Code::Digit5) => Some(4),
        Physical::Code(Code::Digit6) => Some(5),
        Physical::Code(Code::Digit7) => Some(6),
        Physical::Code(Code::Digit8) => Some(7),
        Physical::Code(Code::Digit9) => Some(8),
        _ => None,
    };
    row.or_else(|| {
        let keyboard::Key::Character(typed) = typed else {
            return None;
        };
        let mut chars = typed.chars();
        let digit = chars.next()?.to_digit(10)?;
        (chars.next().is_none() && digit > 0).then(|| digit as usize - 1)
    })
}

struct BeatSubscriptionData(Arc<AtomicU32>, Arc<Notify>);

impl std::hash::Hash for BeatSubscriptionData {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        "beat-subscription".hash(state); // The ID is constant
    }
}

impl PartialEq for BeatSubscriptionData {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl Eq for BeatSubscriptionData {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_keys_pick_the_first_nine_tracks() {
        use keyboard::Key;
        let track = |code, typed| track_of_key(Physical::Code(code), typed);
        assert_eq!(track(Code::Digit1, Key::Character("1")), Some(0));
        // the digit row of a layout that needs Shift for its digits
        assert_eq!(track(Code::Digit2, Key::Character("\u{e9}")), Some(1));
        assert_eq!(track(Code::Digit0, Key::Character("0")), None);
        assert_eq!(track(Code::KeyS, Key::Character("s")), None);
        // the numeric pad types digits with NumLock on, and moves with it off
        assert_eq!(track(Code::Numpad9, Key::Character("9")), Some(8));
        assert_eq!(track(Code::Numpad4, Key::Named(ArrowLeft)), None);
    }
}
