use chrono::prelude::*;

use crate::messages::a2a::{A2AMessage, MessageId};
use crate::messages::localization::Localization;
use crate::messages::thread::Thread;
use crate::messages::timing::Timing;
use crate::timing_optional;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct BasicMessage {
    #[serde(rename = "@id")]
    pub id: MessageId,
    pub sent_time: String,
    pub content: String,
    #[serde(rename = "~l10n")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub l10n: Option<Localization>,
    #[serde(rename = "~thread")]
    pub thread: Option<Thread>,
    #[serde(rename = "~timing")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timing: Option<Timing>,
}

threadlike_optional!(BasicMessage);
timing_optional!(BasicMessage);

impl BasicMessage {
    pub fn create() -> BasicMessage {
        BasicMessage::default()
    }

    pub fn set_content(mut self, content: String) -> Self {
        self.content = content;
        self
    }

    pub fn set_time(mut self) -> Self {
        self.sent_time = format!("{:?}", Utc::now());
        self
    }

    pub fn set_default_localization(mut self) -> Self {
        self.l10n = Some(Localization::default());
        self
    }

    pub fn to_a2a_message(&self) -> A2AMessage {
        A2AMessage::BasicMessage(self.clone()) // TODO: THINK how to avoid clone
    }
}

#[cfg(test)]
#[cfg(feature = "general_test")]
pub mod unit_tests {
    use super::*;

    fn _content() -> String {
        String::from("Your hovercraft is full of eels.")
    }

    #[test]
    fn test_basic_message_build_works() {
        let basic_message: BasicMessage = BasicMessage::default()
            .set_content(_content())
            .set_time()
            .set_default_localization();
        assert_eq!(_content(), basic_message.content);
    }
}
