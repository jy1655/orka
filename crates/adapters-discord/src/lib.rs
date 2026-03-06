use std::sync::Arc;

use anyhow::{bail, Context as AnyhowContext, Result};
use async_trait::async_trait;
use chrono::Utc;
use serenity::all::{
    CommandOptionType, CreateCommand, CreateCommandOption, CreateEmbed, CreateInteractionResponse,
    CreateInteractionResponseMessage, CreateMessage, Interaction,
};
use serenity::client::{Client, Context, EventHandler};
use serenity::http::Http;
use serenity::model::channel::Message;
use serenity::model::gateway::{GatewayIntents, Ready};
use serenity::model::id::ChannelId;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use orka_core::model::{AttachmentMeta, Channel, InboundEvent, OutboundAction};
use orka_core::ports::OutboundSender;
use orka_core::text::{chunk_text, normalize_text};

const MAX_INBOUND_CHARS: usize = 4_000;
const MAX_OUTBOUND_CHARS: usize = 1_900;
const MAX_EMBED_CHARS: usize = 4_000;

pub struct DiscordAdapter {
    token: String,
    inbound_tx: mpsc::Sender<InboundEvent>,
}

struct DiscordHandler {
    inbound_tx: mpsc::Sender<InboundEvent>,
}

#[async_trait]
impl EventHandler for DiscordHandler {
    async fn ready(&self, ctx: Context, ready: Ready) {
        info!(
            "discord adapter ready as {} ({})",
            ready.user.name,
            ready.user.id.get()
        );

        let command = CreateCommand::new("ask")
            .description("Send a prompt to the AI agent")
            .add_option(
                CreateCommandOption::new(
                    CommandOptionType::String,
                    "prompt",
                    "Your question or instruction",
                )
                .required(true),
            );

        if let Err(err) = serenity::all::Command::create_global_command(&ctx.http, command).await {
            error!("failed to register /ask slash command: {err}");
        } else {
            info!("registered /ask global slash command");
        }
    }

    async fn message(&self, _ctx: Context, msg: Message) {
        if msg.author.bot {
            return;
        }

        let Some(text) = normalize_inbound_text(&msg.content) else {
            return;
        };

        let event = InboundEvent {
            idempotency_key: format!("discord:{}:{}", msg.channel_id.get(), msg.id.get()),
            channel: Channel::Discord,
            chat_id: msg.channel_id.get().to_string(),
            user_id: msg.author.id.get().to_string(),
            text,
            received_at: Utc::now(),
            reply_token: None,
            claims: extract_member_claims(&msg.member),
            attachments: extract_attachments(&msg.attachments),
        };

        if let Err(err) = self.inbound_tx.send(event).await {
            warn!("discord inbound dropped: pipeline channel closed ({err})");
        }
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        let Interaction::Command(command) = interaction else {
            return;
        };

        if command.data.name.as_str() != "ask" {
            return;
        }

        let prompt = command
            .data
            .options
            .iter()
            .find(|opt| opt.name == "prompt")
            .and_then(|opt| opt.value.as_str())
            .unwrap_or_default()
            .to_string();

        let Some(text) = normalize_inbound_text(&prompt) else {
            if let Err(err) = command
                .create_response(
                    &ctx.http,
                    CreateInteractionResponse::Message(
                        CreateInteractionResponseMessage::new()
                            .content("Please provide a non-empty prompt.")
                            .ephemeral(true),
                    ),
                )
                .await
            {
                error!("failed to respond to empty /ask: {err}");
            }
            return;
        };

        if let Err(err) = command
            .create_response(
                &ctx.http,
                CreateInteractionResponse::Defer(
                    CreateInteractionResponseMessage::new().content("Processing..."),
                ),
            )
            .await
        {
            error!("failed to defer /ask interaction: {err}");
            return;
        }

        let interaction_token = command.token.clone();
        let application_id = command.application_id.get();

        let event = InboundEvent {
            idempotency_key: format!(
                "discord:interaction:{}:{}",
                command.channel_id.get(),
                command.id.get()
            ),
            channel: Channel::Discord,
            chat_id: command.channel_id.get().to_string(),
            user_id: command.user.id.get().to_string(),
            text,
            received_at: Utc::now(),
            reply_token: Some(format!("{application_id}:{interaction_token}")),
            claims: extract_interaction_claims(&command.member),
            attachments: vec![],
        };

        if let Err(err) = self.inbound_tx.send(event).await {
            warn!("discord interaction inbound dropped: pipeline channel closed ({err})");
        }
    }
}

impl DiscordAdapter {
    pub fn new(token: String, inbound_tx: mpsc::Sender<InboundEvent>) -> Self {
        Self { token, inbound_tx }
    }

    pub fn is_enabled(&self) -> bool {
        !self.token.trim().is_empty()
    }

    pub async fn run(self) -> Result<()> {
        if !self.is_enabled() {
            warn!("discord adapter disabled: DISCORD_BOT_TOKEN is empty");
            return Ok(());
        }

        let intents = GatewayIntents::GUILD_MESSAGES
            | GatewayIntents::DIRECT_MESSAGES
            | GatewayIntents::MESSAGE_CONTENT;
        let handler = DiscordHandler {
            inbound_tx: self.inbound_tx,
        };

        let mut client = Client::builder(self.token, intents)
            .event_handler(handler)
            .await?;

        info!("discord adapter started (gateway mode)");
        client.start().await?;
        Ok(())
    }
}

pub struct DiscordOutbound {
    token: String,
    http: Arc<Http>,
    rest_client: reqwest::Client,
    use_embeds: bool,
}

impl DiscordOutbound {
    pub fn new(token: String) -> Self {
        Self::with_options(token, false)
    }

    pub fn with_options(token: String, use_embeds: bool) -> Self {
        let http = Arc::new(Http::new(&token));
        Self {
            token,
            http,
            rest_client: reqwest::Client::new(),
            use_embeds,
        }
    }

    async fn send_channel_message(&self, channel_id: ChannelId, text: &str) -> Result<()> {
        let max_chars = if self.use_embeds {
            MAX_EMBED_CHARS
        } else {
            MAX_OUTBOUND_CHARS
        };
        let chunks = chunk_text(text, max_chars);
        if chunks.is_empty() {
            channel_id
                .say(self.http.as_ref(), "(empty response)")
                .await?;
            return Ok(());
        }
        for chunk in chunks {
            if self.use_embeds {
                let embed = CreateEmbed::new().description(&chunk).color(0x5865F2);
                let msg = CreateMessage::new().embed(embed);
                channel_id.send_message(self.http.as_ref(), msg).await?;
            } else {
                channel_id.say(self.http.as_ref(), &chunk).await?;
            }
        }
        Ok(())
    }

    async fn send_interaction_response(
        &self,
        application_id: u64,
        interaction_token: &str,
        channel_id: ChannelId,
        text: &str,
    ) -> Result<()> {
        let chunks = chunk_text(text, MAX_OUTBOUND_CHARS);
        let content = if chunks.is_empty() {
            "(empty response)".to_string()
        } else {
            chunks[0].clone()
        };

        let url = format!(
            "https://discord.com/api/v10/webhooks/{application_id}/{interaction_token}/messages/@original"
        );
        let resp = self
            .rest_client
            .patch(&url)
            .json(&serde_json::json!({ "content": content }))
            .send()
            .await
            .context("failed to edit interaction response")?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();

        if !status.is_success() {
            warn!(
                status = %status,
                "interaction edit failed, falling back to channel message: {body}"
            );
            return Err(anyhow::anyhow!(
                "interaction edit failed with status {status}"
            ));
        }

        if chunks.len() > 1 {
            // Try to create a thread from the interaction response message
            let thread_id = self
                .try_create_thread(&body, channel_id)
                .await;

            if let Some(thread_id) = thread_id {
                for chunk in &chunks[1..] {
                    if let Err(err) = thread_id.say(self.http.as_ref(), chunk).await {
                        warn!("failed to send chunk in thread: {err}");
                    }
                }
            } else {
                // Fallback: send followup messages via webhook
                let followup_url = format!(
                    "https://discord.com/api/v10/webhooks/{application_id}/{interaction_token}"
                );
                for chunk in &chunks[1..] {
                    let resp = self
                        .rest_client
                        .post(&followup_url)
                        .json(&serde_json::json!({ "content": chunk }))
                        .send()
                        .await
                        .context("failed to send followup message")?;
                    if !resp.status().is_success() {
                        warn!(
                            status = %resp.status(),
                            "interaction followup failed"
                        );
                    }
                }
            }
        }

        Ok(())
    }

    async fn try_create_thread(
        &self,
        edit_response_body: &str,
        channel_id: ChannelId,
    ) -> Option<ChannelId> {
        let msg: serde_json::Value = serde_json::from_str(edit_response_body).ok()?;
        let message_id = msg.get("id")?.as_str()?.parse::<u64>().ok()?;

        let url = format!(
            "https://discord.com/api/v10/channels/{}/messages/{message_id}/threads",
            channel_id.get()
        );
        let resp = self
            .rest_client
            .post(&url)
            .header("Authorization", format!("Bot {}", self.token))
            .json(&serde_json::json!({
                "name": "Response (continued)",
                "auto_archive_duration": 60,
            }))
            .send()
            .await
            .ok()?;

        if !resp.status().is_success() {
            let status = resp.status();
            warn!(status = %status, "failed to create thread from interaction response");
            return None;
        }

        let thread: serde_json::Value = resp.json().await.ok()?;
        let thread_id = thread.get("id")?.as_str()?.parse::<u64>().ok()?;
        Some(ChannelId::new(thread_id))
    }
}

#[async_trait]
impl OutboundSender for DiscordOutbound {
    async fn send(&self, action: &OutboundAction) -> Result<()> {
        if action.channel != Channel::Discord {
            return Ok(());
        }
        if self.token.trim().is_empty() {
            bail!("discord outbound unavailable: missing DISCORD_BOT_TOKEN");
        }

        let channel_id = parse_channel_id(&action.chat_id)?;

        if let Some(ref reply_token) = action.reply_token {
            if let Some((app_id_str, interaction_token)) = reply_token.split_once(':') {
                if let Ok(app_id) = app_id_str.parse::<u64>() {
                    match self
                        .send_interaction_response(app_id, interaction_token, channel_id, &action.text)
                        .await
                    {
                        Ok(()) => return Ok(()),
                        Err(err) => {
                            warn!("interaction response failed, falling back to channel message: {err}");
                        }
                    }
                }
            }
        }

        self.send_channel_message(channel_id, &action.text).await
    }
}

fn normalize_inbound_text(raw: &str) -> Option<String> {
    normalize_text(raw, MAX_INBOUND_CHARS)
}

fn extract_member_claims(member: &Option<Box<serenity::model::guild::PartialMember>>) -> Vec<String> {
    let Some(member) = member else {
        return vec![];
    };
    member
        .roles
        .iter()
        .map(|role_id| format!("role:{}", role_id.get()))
        .collect()
}

fn extract_interaction_claims(
    member: &Option<Box<serenity::model::guild::Member>>,
) -> Vec<String> {
    let Some(member) = member else {
        return vec![];
    };
    member
        .roles
        .iter()
        .map(|role_id| format!("role:{}", role_id.get()))
        .collect()
}

fn extract_attachments(
    attachments: &[serenity::model::channel::Attachment],
) -> Vec<AttachmentMeta> {
    attachments
        .iter()
        .map(|a| AttachmentMeta {
            filename: a.filename.clone(),
            url: a.url.clone(),
            size_bytes: a.size as u64,
        })
        .collect()
}

fn parse_channel_id(raw: &str) -> Result<ChannelId> {
    let id = raw
        .trim()
        .parse::<u64>()
        .with_context(|| format!("invalid discord chat_id: {raw}"))?;
    Ok(ChannelId::new(id))
}

#[cfg(test)]
mod tests {
    use super::{normalize_inbound_text, parse_channel_id};

    #[test]
    fn inbound_text_is_trimmed_and_empty_ignored() {
        assert_eq!(
            normalize_inbound_text("  hello "),
            Some("hello".to_string())
        );
        assert_eq!(normalize_inbound_text("   "), None);
    }

    #[test]
    fn parse_channel_id_rejects_invalid_value() {
        assert!(parse_channel_id("not-a-number").is_err());
        assert_eq!(
            parse_channel_id("123456789")
                .expect("valid channel id")
                .get(),
            123_456_789
        );
    }
}
