use std::collections::HashSet;

use crate::model::Channel;

#[derive(Debug, Clone)]
pub struct AccessPolicy {
    operators: HashSet<String>,
    open_access: bool,
    public_chat: bool,
    runtime_channels: HashSet<String>,
}

impl AccessPolicy {
    pub fn new(allowlist: impl IntoIterator<Item = String>, open_access: bool) -> Self {
        Self::with_runtime_access(allowlist, open_access, false, Vec::<String>::new())
    }

    pub fn with_runtime_access(
        allowlist: impl IntoIterator<Item = String>,
        open_access: bool,
        public_chat: bool,
        runtime_channels: impl IntoIterator<Item = String>,
    ) -> Self {
        let operators = allowlist
            .into_iter()
            .map(|entry| entry.trim().to_ascii_lowercase())
            .filter(|entry| !entry.is_empty())
            .collect();
        let runtime_channels = runtime_channels
            .into_iter()
            .map(|entry| entry.trim().to_ascii_lowercase())
            .filter(|entry| !entry.is_empty())
            .collect();
        Self {
            operators,
            open_access,
            public_chat,
            runtime_channels,
        }
    }

    pub fn is_operator(&self, channel: Channel, user_id: &str, claims: &[String]) -> bool {
        let user = user_id.trim().to_ascii_lowercase();
        if user.is_empty() {
            return false;
        }
        if self.open_access {
            return true;
        }
        if self.operators.contains(&user) {
            return true;
        }
        let scoped = format!("{}:{user}", channel.as_str());
        if self.operators.contains(&scoped) {
            return true;
        }
        for claim in claims {
            let claim_lower = claim.trim().to_ascii_lowercase();
            if claim_lower.is_empty() {
                continue;
            }
            if self.operators.contains(&claim_lower) {
                return true;
            }
            let scoped_claim = format!("{}:{claim_lower}", channel.as_str());
            if self.operators.contains(&scoped_claim) {
                return true;
            }
        }
        false
    }

    pub fn can_invoke_runtime(
        &self,
        channel: Channel,
        chat_id: &str,
        user_id: &str,
        claims: &[String],
    ) -> bool {
        if self.is_operator(channel, user_id, claims) || self.public_chat {
            return true;
        }

        let channel_key = format!(
            "{}:{}",
            channel.as_str(),
            chat_id.trim().to_ascii_lowercase()
        );
        self.runtime_channels.contains(&channel_key)
    }
}

#[cfg(test)]
mod tests {
    use super::AccessPolicy;
    use crate::model::Channel;

    #[test]
    fn open_access_allows_any_non_empty_user() {
        let policy = AccessPolicy::new(Vec::<String>::new(), true);
        assert!(policy.is_operator(Channel::Discord, "user-1", &[]));
        assert!(!policy.is_operator(Channel::Discord, "   ", &[]));
    }

    #[test]
    fn supports_global_and_scoped_allowlist_entries() {
        let policy = AccessPolicy::new(
            vec![
                "admin-global".to_string(),
                "telegram:admin-scoped".to_string(),
            ],
            false,
        );

        assert!(policy.is_operator(Channel::Discord, "admin-global", &[]));
        assert!(policy.is_operator(Channel::Telegram, "admin-scoped", &[]));
        assert!(!policy.is_operator(Channel::Discord, "admin-scoped", &[]));
    }

    #[test]
    fn allowlist_is_trimmed_and_case_insensitive() {
        let policy = AccessPolicy::new(vec!["  DISCORD:Admin-User  ".to_string()], false);
        assert!(policy.is_operator(Channel::Discord, "admin-user", &[]));
        assert!(policy.is_operator(Channel::Discord, " Admin-User ", &[]));
        assert!(!policy.is_operator(Channel::Telegram, "admin-user", &[]));
    }

    #[test]
    fn role_claim_grants_operator() {
        let policy = AccessPolicy::new(vec!["discord:role:123456".to_string()], false);
        assert!(!policy.is_operator(Channel::Discord, "user-1", &[]));
        assert!(policy.is_operator(Channel::Discord, "user-1", &["role:123456".to_string()]));
        assert!(!policy.is_operator(Channel::Telegram, "user-1", &["role:123456".to_string()]));
    }

    #[test]
    fn global_role_claim_grants_operator_on_any_channel() {
        let policy = AccessPolicy::new(vec!["role:admin".to_string()], false);
        assert!(policy.is_operator(Channel::Discord, "user-1", &["role:admin".to_string()]));
        assert!(policy.is_operator(Channel::Telegram, "user-1", &["role:admin".to_string()]));
    }

    #[test]
    fn runtime_access_defaults_to_operator_or_allowlisted_channel() {
        let policy = AccessPolicy::with_runtime_access(
            vec!["discord:admin".to_string()],
            false,
            false,
            vec!["discord:channel-1".to_string()],
        );

        assert!(policy.can_invoke_runtime(Channel::Discord, "other-channel", "admin", &[]));
        assert!(policy.can_invoke_runtime(Channel::Discord, "channel-1", "user-1", &[]));
        assert!(!policy.can_invoke_runtime(Channel::Discord, "other-channel", "user-1", &[]));
    }

    #[test]
    fn public_chat_allows_runtime_access_without_operator() {
        let policy = AccessPolicy::with_runtime_access(Vec::<String>::new(), false, true, []);

        assert!(policy.can_invoke_runtime(Channel::Telegram, "-100123", "user-1", &[]));
    }
}
