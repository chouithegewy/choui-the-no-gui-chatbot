use iced::futures::SinkExt;
use iced::widget::{column, container, scrollable, text};
use iced::{executor, time, Application, Command, Element, Length, Subscription, Theme};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

use choui_the_no_gui_chatbot::state::AppEvent;

pub struct Overlay {
    messages: Vec<String>,
    alert: Option<(String, std::time::Instant)>,
    receiver: Arc<Mutex<Option<broadcast::Receiver<AppEvent>>>>,
}

#[derive(Debug, Clone)]
pub enum Message {
    EventOccurred(AppEvent),
    Tick(std::time::Instant),
    None,
}

impl Application for Overlay {
    type Executor = executor::Default;
    type Message = Message;
    type Theme = Theme;
    type Flags = Arc<Mutex<Option<broadcast::Receiver<AppEvent>>>>;

    fn new(flags: Self::Flags) -> (Self, Command<Self::Message>) {
        (
            Self {
                messages: Vec::new(),
                alert: None,
                receiver: flags,
            },
            Command::none(),
        )
    }

    fn title(&self) -> String {
        String::from("CHOUIBOT Overlay")
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::EventOccurred(event) => {
                match event {
                    AppEvent::ChatMessage { user, text } => {
                        let msg = format!("{}: {}", user, text);
                        self.messages.push(msg);
                        if self.messages.len() > 20 {
                            self.messages.remove(0);
                        }
                        // TTS is now handled in main.rs (bot thread) so it plays regardless of focus
                    }
                    AppEvent::UserJoined(user) => {
                        self.alert = Some((
                            format!("{} JOINED!", user.to_uppercase()),
                            std::time::Instant::now(),
                        ));
                        // TTS is now handled in main.rs (bot thread) so it plays regardless of focus
                    }
                    _ => {}
                }
            }
            Message::Tick(now) => {
                if let Some((_, start)) = self.alert {
                    if now.duration_since(start).as_secs() > 5 {
                        self.alert = None;
                    }
                }
            }
            Message::None => {}
        }
        Command::none()
    }

    fn view(&self) -> Element<Message> {
        let chat_log = scrollable(
            column(
                self.messages
                    .iter()
                    .map(|msg| text(msg).size(22).style(iced::Color::WHITE).into())
                    .collect::<Vec<_>>(),
            )
            .spacing(8),
        )
        .height(Length::Fill);

        let chat_container = container(chat_log)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(10)
            .style(iced::theme::Container::Custom(Box::new(
                ChatBackgroundStyle,
            )));

        let content = if let Some((alert_text, _)) = &self.alert {
            column![
                container(
                    text(alert_text)
                        .size(48)
                        .style(iced::Color::from_rgb(1.0, 0.3, 0.3))
                )
                .padding(20)
                .style(iced::theme::Container::Custom(Box::new(AlertStyle))),
                chat_container
            ]
        } else {
            column![chat_container]
        };

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(15)
            .style(iced::theme::Container::Custom(Box::new(TransparentStyle)))
            .into()
    }

    fn subscription(&self) -> Subscription<Message> {
        let tick = time::every(std::time::Duration::from_millis(100)).map(Message::Tick);

        struct EventLoop;

        let receiver_arc = self.receiver.clone();

        let events = iced::subscription::channel(
            std::any::TypeId::of::<EventLoop>(),
            100,
            move |mut output| {
                // move receiver_arc in
                let receiver_arc = receiver_arc.clone();
                async move {
                    let mut rx_opt = receiver_arc.lock().unwrap().take();
                    if let Some(mut rx) = rx_opt {
                        loop {
                            match rx.recv().await {
                                Ok(event) => {
                                    let _ = output.send(Message::EventOccurred(event)).await;
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                    continue;
                                }
                                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                                    break;
                                }
                            }
                        }
                    }

                    // Diverge forever
                    loop {
                        std::future::pending::<()>().await;
                    }
                }
            },
        );

        Subscription::batch(vec![tick, events])
    }
}

struct AlertStyle;
impl container::StyleSheet for AlertStyle {
    type Style = Theme;

    fn appearance(&self, _style: &Self::Style) -> container::Appearance {
        container::Appearance {
            text_color: Some(iced::Color::WHITE),
            background: Some(iced::Color::BLACK.into()),
            border: iced::Border {
                color: iced::Color::from_rgb(1.0, 0.0, 0.0),
                width: 2.0,
                radius: 5.0.into(),
            },
            shadow: iced::Shadow::default(),
        }
    }
}

struct TransparentStyle;
impl container::StyleSheet for TransparentStyle {
    type Style = Theme;

    fn appearance(&self, _style: &Self::Style) -> container::Appearance {
        container::Appearance {
            text_color: None,
            background: None, // Fully transparent
            border: iced::Border::default(),
            shadow: iced::Shadow::default(),
        }
    }
}

struct ChatBackgroundStyle;
impl container::StyleSheet for ChatBackgroundStyle {
    type Style = Theme;

    fn appearance(&self, _style: &Self::Style) -> container::Appearance {
        container::Appearance {
            text_color: Some(iced::Color::WHITE),
            background: Some(iced::Color::from_rgba(0.0, 0.0, 0.0, 0.7).into()), // Semi-transparent dark
            border: iced::Border {
                color: iced::Color::from_rgba(1.0, 1.0, 1.0, 0.2),
                width: 1.0,
                radius: 8.0.into(),
            },
            shadow: iced::Shadow::default(),
        }
    }
}
