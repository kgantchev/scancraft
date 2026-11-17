//! Quitting while a PDF is being written (§4.8): one question, asked by the system as an alert on
//! the window, and what each answer means.
//!
//! The question used to be a window drawn inside the app, which a screen reader reads as part of
//! the page and which looks like nothing else on the Mac. The system's alert is a sheet the
//! system announces, and has the keyboard's usual defaults.

use craft_dialogs::{Answer, Message, MessageButtons, MessageLevel, MessageResult};

/// The button that goes on exporting. It is the first, so it is the one Return presses.
pub const KEEP: &str = "Keep Exporting";
/// The button that gives the export up and quits.
pub const QUIT: &str = "Cancel Export";

/// The alert.
pub fn question() -> Message {
    Message {
        level: MessageLevel::Warning,
        title: "A PDF is still being written".to_owned(),
        description: "Quit now and the PDF is not written. Your scan is kept, and you can export it again.".to_owned(),
        buttons: MessageButtons::OkCancelCustom(KEEP.to_owned(), QUIT.to_owned()),
    }
}

/// What the answer means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Go on exporting; the app stays open.
    Keep,
    /// Cancel the export and quit.
    Quit,
}

/// Only the button that says so quits. Anything else, the other button, Escape, a closed alert or
/// an alert that could not be shown, goes on exporting: losing a PDF takes a clear yes.
pub fn decide(answer: &Answer) -> Decision {
    match answer {
        Answer::Message(MessageResult::Custom(label)) if label == QUIT => Decision::Quit,
        _ => Decision::Keep,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_alert_offers_keep_first_and_cancel_second_with_a_warning() {
        let message = question();
        assert_eq!(message.level, MessageLevel::Warning);
        assert_eq!(message.buttons, MessageButtons::OkCancelCustom(KEEP.to_owned(), QUIT.to_owned()));
    }

    #[test]
    fn only_the_cancel_button_quits() {
        assert_eq!(decide(&Answer::Message(MessageResult::Custom(QUIT.to_owned()))), Decision::Quit);
        for answer in [
            Answer::Message(MessageResult::Custom(KEEP.to_owned())),
            Answer::Message(MessageResult::Ok),
            Answer::Message(MessageResult::Cancel),
            Answer::Message(MessageResult::No),
            Answer::Cancelled,
            Answer::Paths(vec![]),
            Answer::Unavailable("no window".to_owned()),
        ] {
            assert_eq!(decide(&answer), Decision::Keep, "{answer:?}");
        }
    }
}
