mod controller;
pub use self::controller::Controller;

mod dispatcher;
pub use self::dispatcher::Dispatcher;

use crate::UIEventChannel;
use futures::channel::oneshot;

#[derive(Debug)]
pub enum Event {
    AskQuestion {
        question: String,
        response_sender: oneshot::Sender<gtk::ResponseType>,
    },
    Hide,
    ShowError(String),
    ShowInfo(String),
}

pub async fn ask_question(question: impl ToString) -> gtk::ResponseType {
    let (response_sender, response_receiver) = oneshot::channel();
    UIEventChannel::send(Event::AskQuestion {
        question: question.to_string(),
        response_sender,
    });

    response_receiver.await.unwrap_or(gtk::ResponseType::Cancel)
}

pub fn hide() {
    UIEventChannel::send(Event::Hide);
}

pub fn show_error(error: impl ToString) {
    UIEventChannel::send(Event::ShowError(error.to_string()));
}

pub fn show_info(info: impl ToString) {
    UIEventChannel::send(Event::ShowInfo(info.to_string()));
}
