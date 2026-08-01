//! DNS Rewrite Engine
//!
//! Provides domain name rewriting functionality with support for:
//! - Exact matching
//! - Wildcard matching (*.example.com)
//! - Regular expression matching
//!
//! Supports rewrite actions:
//! - Map to IP address
//! - Map to another domain
//! - Block (return NXDOMAIN)

use std::net::IpAddr;
use std::sync::Arc;

use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::infrastructure::repository::{Database, RewriteRule as DbRewriteRule};

/// Match type for rewrite rules
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchType {
    /// Exact domain match
    Exact,
    /// Wildcard match (*.example.com)
    Wildcard,
    /// Regular expression match
    Regex,
}

#[allow(dead_code)]
impl MatchType {
    /// Parse from string
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "exact" => Some(MatchType::Exact),
            "wildcard" => Some(MatchType::Wildcard),
            "regex" => Some(MatchType::Regex),
            _ => None,
        }
    }

    /// Convert to string
    pub fn as_str(&self) -> &'static str {
        match self {
            MatchType::Exact => "exact",
            MatchType::Wildcard => "wildcard",
            MatchType::Regex => "regex",
        }
    }
}

/// Rewrite action to perform when a rule matches
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum RewriteAction {
    /// Map to a specific IP address
    MapToIp(IpAddr),
    /// Map to another domain name
    MapToDomain(String),
    /// Block the request (return NXDOMAIN)
    Block,
}

#[allow(dead_code)]
impl RewriteAction {
    /// Parse action type and value from strings
    pub fn from_parts(action_type: &str, action_value: Option<&str>) -> Option<Self> {
        match action_type.to_lowercase().as_str() {
            "map_ip" | "maptoip" => {
                let ip: IpAddr = action_value?.parse().ok()?;
                Some(RewriteAction::MapToIp(ip))
            }
            "map_domain" | "maptodomain" => {
                Some(RewriteAction::MapToDomain(action_value?.to_string()))
            }
            "block" => Some(RewriteAction::Block),
            _ => None,
        }
    }

    /// Get the action type as string
    pub fn action_type(&self) -> &'static str {
        match self {
            RewriteAction::MapToIp(_) => "map_ip",
            RewriteAction::MapToDomain(_) => "map_domain",
            RewriteAction::Block => "block",
        }
    }

    /// Get the action value as string
    pub fn action_value(&self) -> Option<String> {
        match self {
            RewriteAction::MapToIp(ip) => Some(ip.to_string()),
            RewriteAction::MapToDomain(domain) => Some(domain.clone()),
            RewriteAction::Block => None,
        }
    }
}

/// A compiled rewrite rule
#[derive(Debug, Clone)]
pub struct RewriteRule {
    /// Rule ID from database
    pub id: i64,
    /// Match type
    pub match_type: MatchType,
    /// Action to perform
    pub action: RewriteAction,
    /// Whether the rule is enabled
    pub enabled: bool,
    /// Priority (higher = checked first)
    pub priority: i32,
    /// Compiled regex (for regex match type)
    compiled_regex: Option<Regex>,
    /// Lowercased pattern, precomputed for matching.
    ///
    /// Matching is case-insensitive, and lowercasing the pattern on every call
    /// meant one allocation per rule per query while the pattern itself never
    /// changes.
    pattern_lower: String,
}

#[allow(dead_code)]
impl RewriteRule {
    /// Create a new rewrite rule
    pub fn new(
        id: i64,
        pattern: String,
        match_type: MatchType,
        action: RewriteAction,
        priority: i32,
    ) -> Self {
        let compiled_regex = if match_type == MatchType::Regex {
            Regex::new(&pattern).ok()
        } else {
            None
        };

        Self {
            pattern_lower: pattern.to_lowercase(),
            id,
            match_type,
            action,
            enabled: true,
            priority,
            compiled_regex,
        }
    }

    /// Create from database model
    pub fn from_db(db_rule: &DbRewriteRule) -> Option<Self> {
        let match_type = MatchType::from_str(&db_rule.match_type)?;
        let action =
            RewriteAction::from_parts(&db_rule.action_type, db_rule.action_value.as_deref())?;

        let compiled_regex = if match_type == MatchType::Regex {
            Regex::new(&db_rule.pattern).ok()
        } else {
            None
        };

        Some(Self {
            id: db_rule.id,
            pattern_lower: db_rule.pattern.to_lowercase(),
            match_type,
            action,
            enabled: db_rule.enabled,
            priority: db_rule.priority,
            compiled_regex,
        })
    }

    /// Check if this rule matches the given domain
    pub fn matches(&self, domain: &str) -> bool {
        self.matches_lowercase(&domain.to_lowercase())
    }

    /// Check a domain that the caller has already lowercased.
    ///
    /// `RewriteEngine::check` walks every rule for one domain, so lowercasing is
    /// hoisted out of the loop and done once per query instead of once per rule.
    pub fn matches_lowercase(&self, domain_lower: &str) -> bool {
        if !self.enabled {
            return false;
        }

        match self.match_type {
            MatchType::Exact => domain_lower == self.pattern_lower,
            MatchType::Wildcard => self.wildcard_matches(domain_lower, &self.pattern_lower),
            MatchType::Regex => self.regex_matches(domain_lower),
        }
    }

    /// Check wildcard match
    fn wildcard_matches(&self, domain: &str, pattern: &str) -> bool {
        if pattern.starts_with("*.") {
            // *.example.com should match sub.example.com but NOT example.com
            let suffix = &pattern[1..]; // .example.com
            if domain.ends_with(suffix) {
                // Make sure there's something before the suffix
                let prefix_len = domain.len() - suffix.len();
                prefix_len > 0 && !domain[..prefix_len].contains('.') || prefix_len > 0
            }
            // Actually, *.example.com should match ANY subdomain
            else {
                false
            }
        } else if pattern.contains('*') {
            // General wildcard pattern
            let parts: Vec<&str> = pattern.split('*').collect();
            if parts.len() == 2 {
                domain.starts_with(parts[0]) && domain.ends_with(parts[1])
            } else {
                false
            }
        } else {
            // No wildcard, treat as exact match
            domain == pattern
        }
    }

    /// Check regex match
    fn regex_matches(&self, domain: &str) -> bool {
        if let Some(ref regex) = self.compiled_regex {
            regex.is_match(domain)
        } else {
            false
        }
    }
}

/// Result of a rewrite operation
#[derive(Debug, Clone)]
pub struct RewriteResult {
    /// The rule that matched
    pub rule_id: i64,
    /// The action to perform
    pub action: RewriteAction,
}

/// DNS Rewrite Engine
///
/// Manages rewrite rules and performs domain rewriting.
pub struct RewriteEngine {
    /// Loaded rules (sorted by priority, highest first)
    rules: RwLock<Vec<RewriteRule>>,
    /// Database connection for persistence
    db: Option<Arc<Database>>,
}

#[allow(dead_code)]
impl RewriteEngine {
    /// Create a new rewrite engine without database
    pub fn new() -> Self {
        Self {
            rules: RwLock::new(Vec::new()),
            db: None,
        }
    }

    /// Create a new rewrite engine with database connection
    pub fn with_db(db: Arc<Database>) -> Self {
        Self {
            rules: RwLock::new(Vec::new()),
            db: Some(db),
        }
    }

    /// Create a new rewrite engine wrapped in Arc
    pub fn new_shared() -> Arc<Self> {
        Arc::new(Self::new())
    }

    /// Load rules from database
    pub async fn load_rules(&self) -> anyhow::Result<()> {
        if let Some(ref db) = self.db {
            let db_rules = db.rewrite_rules().list().await?;
            let mut rules: Vec<RewriteRule> = db_rules
                .iter()
                .filter_map(|r| RewriteRule::from_db(r))
                .collect();

            // Sort by priority (highest first)
            rules.sort_by(|a, b| b.priority.cmp(&a.priority));

            let mut current_rules = self.rules.write().await;
            *current_rules = rules;
        }
        Ok(())
    }

    /// Reload rules from database
    pub async fn reload_rules(&self) -> anyhow::Result<()> {
        self.load_rules().await
    }

    /// Check if a domain matches any rewrite rule
    /// Check if a domain matches any rewrite rule
    ///
    /// The domain is lowercased once here rather than inside each rule's match,
    /// so rule count no longer multiplies the allocation cost of a query.
    pub async fn check(&self, domain: &str) -> Option<RewriteResult> {
        let domain_lower = domain.to_lowercase();
        let rules = self.rules.read().await;

        for rule in rules.iter() {
            if rule.matches_lowercase(&domain_lower) {
                return Some(RewriteResult {
                    rule_id: rule.id,
                    action: rule.action.clone(),
                });
            }
        }

        None
    }

    /// Add a rule (in-memory only, use database for persistence)
    pub async fn add_rule(&self, rule: RewriteRule) {
        let mut rules = self.rules.write().await;
        rules.push(rule);
        rules.sort_by(|a, b| b.priority.cmp(&a.priority));
    }

    /// Remove a rule by ID
    pub async fn remove_rule(&self, id: i64) {
        let mut rules = self.rules.write().await;
        rules.retain(|r| r.id != id);
    }

    /// Get all rules
    pub async fn list_rules(&self) -> Vec<RewriteRule> {
        self.rules.read().await.clone()
    }

    /// Clear all rules
    pub async fn clear_rules(&self) {
        let mut rules = self.rules.write().await;
        rules.clear();
    }

    /// Get the number of rules
    pub async fn rule_count(&self) -> usize {
        self.rules.read().await.len()
    }
}

impl Default for RewriteEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn test_match_type_from_str() {
        assert_eq!(MatchType::from_str("exact"), Some(MatchType::Exact));
        assert_eq!(MatchType::from_str("WILDCARD"), Some(MatchType::Wildcard));
        assert_eq!(MatchType::from_str("Regex"), Some(MatchType::Regex));
        assert_eq!(MatchType::from_str("invalid"), None);
    }

    #[test]
    fn test_rewrite_action_from_parts() {
        let ip_action = RewriteAction::from_parts("map_ip", Some("192.168.1.1"));
        assert!(matches!(ip_action, Some(RewriteAction::MapToIp(_))));

        let domain_action = RewriteAction::from_parts("map_domain", Some("example.com"));
        assert!(matches!(domain_action, Some(RewriteAction::MapToDomain(_))));

        let block_action = RewriteAction::from_parts("block", None);
        assert!(matches!(block_action, Some(RewriteAction::Block)));

        let invalid = RewriteAction::from_parts("invalid", None);
        assert!(invalid.is_none());
    }

    #[test]
    fn test_exact_match() {
        let rule = RewriteRule::new(
            1,
            "example.com".to_string(),
            MatchType::Exact,
            RewriteAction::Block,
            0,
        );

        assert!(rule.matches("example.com"));
        assert!(rule.matches("EXAMPLE.COM"));
        assert!(!rule.matches("sub.example.com"));
        assert!(!rule.matches("example.org"));
    }

    #[test]
    fn test_wildcard_match() {
        let rule = RewriteRule::new(
            1,
            "*.example.com".to_string(),
            MatchType::Wildcard,
            RewriteAction::Block,
            0,
        );

        assert!(rule.matches("sub.example.com"));
        assert!(rule.matches("www.example.com"));
        assert!(rule.matches("deep.sub.example.com"));
        assert!(!rule.matches("example.com"));
        assert!(!rule.matches("example.org"));
    }

    #[test]
    fn test_regex_match() {
        let rule = RewriteRule::new(
            1,
            r"^ads?\..*\.com$".to_string(),
            MatchType::Regex,
            RewriteAction::Block,
            0,
        );

        assert!(rule.matches("ad.example.com"));
        assert!(rule.matches("ads.tracker.com"));
        assert!(!rule.matches("example.com"));
        assert!(!rule.matches("ad.example.org"));
    }

    #[test]
    fn test_disabled_rule() {
        let mut rule = RewriteRule::new(
            1,
            "example.com".to_string(),
            MatchType::Exact,
            RewriteAction::Block,
            0,
        );
        rule.enabled = false;

        assert!(!rule.matches("example.com"));
    }

    #[test]
    fn test_action_type_and_value() {
        let ip_action = RewriteAction::MapToIp(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)));
        assert_eq!(ip_action.action_type(), "map_ip");
        assert_eq!(ip_action.action_value(), Some("192.168.1.1".to_string()));

        let domain_action = RewriteAction::MapToDomain("example.com".to_string());
        assert_eq!(domain_action.action_type(), "map_domain");
        assert_eq!(
            domain_action.action_value(),
            Some("example.com".to_string())
        );

        let block_action = RewriteAction::Block;
        assert_eq!(block_action.action_type(), "block");
        assert_eq!(block_action.action_value(), None);
    }

    #[tokio::test]
    async fn test_rewrite_engine_check() {
        let engine = RewriteEngine::new();

        engine
            .add_rule(RewriteRule::new(
                1,
                "blocked.com".to_string(),
                MatchType::Exact,
                RewriteAction::Block,
                10,
            ))
            .await;

        engine
            .add_rule(RewriteRule::new(
                2,
                "*.ads.com".to_string(),
                MatchType::Wildcard,
                RewriteAction::MapToIp(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))),
                5,
            ))
            .await;

        let result = engine.check("blocked.com").await;
        assert!(result.is_some());
        assert!(matches!(result.unwrap().action, RewriteAction::Block));

        let result = engine.check("tracker.ads.com").await;
        assert!(result.is_some());
        assert!(matches!(result.unwrap().action, RewriteAction::MapToIp(_)));

        let result = engine.check("allowed.com").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_rewrite_engine_priority() {
        let engine = RewriteEngine::new();

        // Lower priority rule
        engine
            .add_rule(RewriteRule::new(
                1,
                "*.example.com".to_string(),
                MatchType::Wildcard,
                RewriteAction::MapToDomain("fallback.com".to_string()),
                1,
            ))
            .await;

        // Higher priority rule
        engine
            .add_rule(RewriteRule::new(
                2,
                "special.example.com".to_string(),
                MatchType::Exact,
                RewriteAction::Block,
                10,
            ))
            .await;

        // Should match higher priority rule
        let result = engine.check("special.example.com").await;
        assert!(result.is_some());
        assert_eq!(result.unwrap().rule_id, 2);
    }

    #[tokio::test]
    async fn test_rewrite_engine_remove_rule() {
        let engine = RewriteEngine::new();

        engine
            .add_rule(RewriteRule::new(
                1,
                "example.com".to_string(),
                MatchType::Exact,
                RewriteAction::Block,
                0,
            ))
            .await;

        assert_eq!(engine.rule_count().await, 1);

        engine.remove_rule(1).await;

        assert_eq!(engine.rule_count().await, 0);
    }

    #[tokio::test]
    async fn test_rewrite_engine_clear() {
        let engine = RewriteEngine::new();

        engine
            .add_rule(RewriteRule::new(
                1,
                "a.com".to_string(),
                MatchType::Exact,
                RewriteAction::Block,
                0,
            ))
            .await;

        engine
            .add_rule(RewriteRule::new(
                2,
                "b.com".to_string(),
                MatchType::Exact,
                RewriteAction::Block,
                0,
            ))
            .await;

        engine.clear_rules().await;

        assert_eq!(engine.rule_count().await, 0);
    }

    #[test]
    fn test_ipv6_action() {
        let ip = IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1));
        let action = RewriteAction::MapToIp(ip);
        assert_eq!(action.action_value(), Some("::1".to_string()));
    }
}

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;
    use std::net::Ipv4Addr;

    /// Strategy to generate valid domain labels
    fn label_strategy() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9]{0,9}".prop_map(|s| s)
    }

    /// Strategy to generate valid domain names
    fn domain_strategy() -> impl Strategy<Value = String> {
        (label_strategy(), label_strategy()).prop_map(|(l1, l2)| format!("{}.{}", l1, l2))
    }

    /// Strategy to generate rewrite actions
    fn action_strategy() -> impl Strategy<Value = RewriteAction> {
        prop_oneof![
            (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>()).prop_map(|(a, b, c, d)| {
                RewriteAction::MapToIp(IpAddr::V4(Ipv4Addr::new(a, b, c, d)))
            }),
            domain_strategy().prop_map(|d| RewriteAction::MapToDomain(d)),
            Just(RewriteAction::Block),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]

        /// Property 10: 重写规则匹配正确性 - Exact Match
        /// For any exact match rule, only the exact domain (case-insensitive) should match.
        /// **Validates: Requirements 8.2**
        #[test]
        fn prop_exact_match_correctness(
            domain in domain_strategy(),
            action in action_strategy(),
            priority in any::<i32>()
        ) {
            let rule = RewriteRule::new(
                1,
                domain.clone(),
                MatchType::Exact,
                action,
                priority,
            );

            // Exact match should work (case-insensitive)
            prop_assert!(rule.matches(&domain), "Exact match should match the domain");
            prop_assert!(rule.matches(&domain.to_uppercase()), "Exact match should be case-insensitive");

            // Subdomain should NOT match
            let subdomain = format!("sub.{}", domain);
            prop_assert!(!rule.matches(&subdomain), "Exact match should not match subdomains");

            // Different domain should NOT match
            let different = format!("{}x", domain);
            prop_assert!(!rule.matches(&different), "Exact match should not match different domains");
        }

        /// Property 10: 重写规则匹配正确性 - Wildcard Match
        /// For any wildcard rule *.domain.tld, subdomains should match but not the base domain.
        /// **Validates: Requirements 8.3**
        #[test]
        fn prop_wildcard_match_correctness(
            base_domain in domain_strategy(),
            subdomain_label in label_strategy(),
            action in action_strategy(),
            priority in any::<i32>()
        ) {
            let pattern = format!("*.{}", base_domain);
            let rule = RewriteRule::new(
                1,
                pattern,
                MatchType::Wildcard,
                action,
                priority,
            );

            // Subdomain should match
            let subdomain = format!("{}.{}", subdomain_label, base_domain);
            prop_assert!(rule.matches(&subdomain), "Wildcard should match subdomains");

            // Base domain should NOT match
            prop_assert!(!rule.matches(&base_domain), "Wildcard *.domain should not match base domain");
        }

        /// Property 10: 重写规则匹配正确性 - Regex Match
        /// For any valid regex pattern, matching should follow regex semantics.
        /// **Validates: Requirements 8.4**
        #[test]
        fn prop_regex_match_correctness(
            domain in domain_strategy(),
            action in action_strategy(),
            priority in any::<i32>()
        ) {
            // Create a regex that matches the exact domain
            let pattern = format!("^{}$", regex::escape(&domain));
            let rule = RewriteRule::new(
                1,
                pattern,
                MatchType::Regex,
                action,
                priority,
            );

            // Should match the domain
            prop_assert!(rule.matches(&domain), "Regex should match the target domain");

            // Should NOT match a different domain
            let different = format!("x{}", domain);
            prop_assert!(!rule.matches(&different), "Regex should not match different domains");
        }

        /// Property 11: 重写动作执行正确性 - Action Type Preservation
        /// For any rewrite action, the action type and value should be correctly preserved.
        /// **Validates: Requirements 8.5, 8.6, 8.7**
        #[test]
        fn prop_action_preservation(
            ip_octets in (any::<u8>(), any::<u8>(), any::<u8>(), any::<u8>()),
            target_domain in domain_strategy()
        ) {
            // MapToIp action
            let ip = IpAddr::V4(Ipv4Addr::new(ip_octets.0, ip_octets.1, ip_octets.2, ip_octets.3));
            let ip_action = RewriteAction::MapToIp(ip);
            prop_assert_eq!(ip_action.action_type(), "map_ip");
            prop_assert_eq!(ip_action.action_value(), Some(ip.to_string()));

            // MapToDomain action
            let domain_action = RewriteAction::MapToDomain(target_domain.clone());
            prop_assert_eq!(domain_action.action_type(), "map_domain");
            prop_assert_eq!(domain_action.action_value(), Some(target_domain));

            // Block action
            let block_action = RewriteAction::Block;
            prop_assert_eq!(block_action.action_type(), "block");
            prop_assert!(block_action.action_value().is_none());
        }

        /// Property 11: Action round-trip through from_parts
        /// For any action, converting to parts and back should preserve the action.
        /// **Validates: Requirements 8.5, 8.6, 8.7**
        #[test]
        fn prop_action_roundtrip(action in action_strategy()) {
            let action_type = action.action_type();
            let action_value = action.action_value();

            let reconstructed = RewriteAction::from_parts(action_type, action_value.as_deref());
            prop_assert!(reconstructed.is_some(), "Action should be reconstructable from parts");

            let reconstructed = reconstructed.unwrap();
            prop_assert_eq!(reconstructed.action_type(), action.action_type());
            prop_assert_eq!(reconstructed.action_value(), action.action_value());
        }

        /// Property 12: 重写规则优先级正确性
        /// For any set of rules with different priorities, higher priority rules should match first.
        /// **Validates: Requirements 8.1**
        #[test]
        fn prop_priority_ordering(
            domain in domain_strategy(),
            low_priority in 0i32..50i32,
            high_priority in 50i32..100i32
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let engine = RewriteEngine::new();

                // Add low priority rule first
                engine.add_rule(RewriteRule::new(
                    1,
                    domain.clone(),
                    MatchType::Exact,
                    RewriteAction::MapToDomain("low.com".to_string()),
                    low_priority,
                )).await;

                // Add high priority rule second
                engine.add_rule(RewriteRule::new(
                    2,
                    domain.clone(),
                    MatchType::Exact,
                    RewriteAction::Block,
                    high_priority,
                )).await;

                // High priority rule should match
                let result = engine.check(&domain).await;
                prop_assert!(result.is_some(), "Should match a rule");
                prop_assert_eq!(result.unwrap().rule_id, 2, "Higher priority rule should match first");

                Ok(())
            })?;
        }

        /// Property 12: Rules with same priority maintain insertion order
        /// **Validates: Requirements 8.1**
        #[test]
        fn prop_same_priority_order(
            domain in domain_strategy(),
            priority in any::<i32>()
        ) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let engine = RewriteEngine::new();

                // Add first rule
                engine.add_rule(RewriteRule::new(
                    1,
                    domain.clone(),
                    MatchType::Exact,
                    RewriteAction::Block,
                    priority,
                )).await;

                // Add second rule with same priority
                engine.add_rule(RewriteRule::new(
                    2,
                    domain.clone(),
                    MatchType::Exact,
                    RewriteAction::MapToDomain("other.com".to_string()),
                    priority,
                )).await;

                // First rule should match (stable ordering)
                let result = engine.check(&domain).await;
                prop_assert!(result.is_some(), "Should match a rule");
                // Note: With same priority, the first added rule should match
                prop_assert_eq!(result.unwrap().rule_id, 1, "First rule with same priority should match");

                Ok(())
            })?;
        }

        /// Property 10: Disabled rules should not match
        /// **Validates: Requirements 8.1-8.4**
        #[test]
        fn prop_disabled_rule_no_match(
            domain in domain_strategy(),
            action in action_strategy(),
            priority in any::<i32>()
        ) {
            let mut rule = RewriteRule::new(
                1,
                domain.clone(),
                MatchType::Exact,
                action,
                priority,
            );
            rule.enabled = false;

            prop_assert!(!rule.matches(&domain), "Disabled rules should not match");
        }
    }
}
