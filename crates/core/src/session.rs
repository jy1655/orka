use crate::model::{Channel, InboundEvent};

pub fn session_key(channel: Channel, chat_id: &str) -> String {
    format!("{}:{}", channel.as_str(), chat_id.trim())
}

pub fn session_key_for_event(event: &InboundEvent) -> String {
    session_key(event.channel, &event.chat_id)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::{session_key, session_key_for_event};
    use crate::model::{Channel, InboundEvent};

    #[test]
    fn session_key_trims_chat_id() {
        assert_eq!(
            session_key(Channel::Discord, " 12345 "),
            "discord:12345".to_string()
        );
    }

    #[test]
    fn session_key_for_event_uses_event_channel_and_chat_id() {
        let event = InboundEvent {
            idempotency_key: "evt-1".to_string(),
            channel: Channel::Telegram,
            chat_id: " 777 ".to_string(),
            user_id: "u-1".to_string(),
            text: "hello".to_string(),
            received_at: Utc::now(),
            is_direct_message: false,
            reply_token: None,
            claims: vec![],
            attachments: vec![],
        };
        assert_eq!(session_key_for_event(&event), "telegram:777".to_string());
    }
}
