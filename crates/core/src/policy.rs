use std::collections::HashSet;

use crate::model::Channel;

#[derive(Debug, Clone)]
pub struct AccessPolicy {
    operators: HashSet<String>,
    open_access: bool,
}

impl AccessPolicy {
    pub fn new(allowlist: impl IntoIterator<Item = String>, open_access: bool) -> Self {
        let operators = allowlist
            .into_iter()
            .map(|entry| entry.trim().to_ascii_lowercase())
            .filter(|entry| !entry.is_empty())
            .collect();
        Self {
            operators,
            open_access,
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
        let policy = AccessPolicy::new(
            vec!["discord:role:123456".to_string()],
            false,
        );
        assert!(!policy.is_operator(Channel::Discord, "user-1", &[]));
        assert!(policy.is_operator(
            Channel::Discord,
            "user-1",
            &["role:123456".to_string()]
        ));
        assert!(!policy.is_operator(
            Channel::Telegram,
            "user-1",
            &["role:123456".to_string()]
        ));
    }

    #[test]
    fn global_role_claim_grants_operator_on_any_channel() {
        let policy = AccessPolicy::new(
            vec!["role:admin".to_string()],
            false,
        );
        assert!(policy.is_operator(
            Channel::Discord,
            "user-1",
            &["role:admin".to_string()]
        ));
        assert!(policy.is_operator(
            Channel::Telegram,
            "user-1",
            &["role:admin".to_string()]
        ));
    }
}
