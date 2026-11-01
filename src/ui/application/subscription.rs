//! The event sources the application listens to: keyboard, playback ticks,
//! window events and the end of a selection drag.

use iced::{Subscription, event, keyboard, mouse, stream, window};

use iced::futures::{SinkExt, Stream};
use iced::keyboard::key::Named::{ArrowDown, ArrowLeft, ArrowRight, ArrowUp, F11, Space};
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
                modifiers,
                ..
            } = event
            else {
                return None;
            };
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

        let window_resized = window::resize_events().map(|_| Message::WindowResized);
        subscriptions.push(window_resized);

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
