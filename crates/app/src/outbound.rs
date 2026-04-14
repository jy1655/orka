use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use orka_adapters_discord::DiscordOutbound;
use orka_adapters_telegram::TelegramOutbound;
use orka_core::model::{Channel, OutboundAction};
use orka_core::ports::OutboundSender;

pub struct MultiplexOutbound {
    discord: Arc<DiscordOutbound>,
    telegram: Arc<TelegramOutbound>,
}

impl MultiplexOutbound {
    pub fn new(discord: Arc<DiscordOutbound>, telegram: Arc<TelegramOutbound>) -> Self {
        Self { discord, telegram }
    }
}

#[async_trait]
impl OutboundSender for MultiplexOutbound {
    async fn send(&self, action: &OutboundAction) -> Result<()> {
        match action.channel {
            Channel::Discord => self.discord.send(action).await,
            Channel::Telegram => self.telegram.send(action).await,
        }
    }

    async fn send_typing(&self, channel: Channel, chat_id: &str) -> Result<()> {
        match channel {
            Channel::Discord => self.discord.send_typing(channel, chat_id).await,
            Channel::Telegram => self.telegram.send_typing(channel, chat_id).await,
        }
    }
}
