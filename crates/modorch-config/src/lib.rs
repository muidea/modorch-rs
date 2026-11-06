//! Optional strict TOML adapter for explicit application composition.
//!
//! # Ownership and errors
//! The host loads files, merges environment and maps parsed values into ApplicationBuilder.
//! Parsing rejects unknown settings, invalid limits and unsupported publication policy.
//! A configuration entry alone cannot register an unavailable component Factory.
//! Contract: COMP-002, DEP-001, COMPAT-002.
//!
//! # Examples
//!
//! ```
//! use modorch_config::CompositionConfig;
//! let config = CompositionConfig::parse(r#"
//! [application]
//! name = "documented-app"
//! [modules.orders]
//! enabled = true
//! "#).unwrap();
//! assert_eq!(config.selection().unwrap().components().count(), 1);
//! ```
//!
//! # Stability
//! Development API 0.0.3, accessed through modorch::config with config-toml enabled.
//! There is no implicit file, signal, global configuration or network lifecycle.
#![forbid(unsafe_code)]
#![deny(missing_docs)]
use modorch_core::*;
use modorch_registry::ComponentSelection;
use serde::Deserialize;
use std::{collections::BTreeMap, time::Duration};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
/// Strict optional TOML composition schema; file loading and environment merging belong to the host.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct CompositionConfig {
    /// Application composition settings.
    pub application: ApplicationConfig,
    #[serde(default)]
    /// Initiator enable/config entries keyed by component name.
    pub initiators: BTreeMap<String, ComponentSettings>,
    #[serde(default)]
    /// Module enable/config entries keyed by component name.
    pub modules: BTreeMap<String, ComponentSettings>,
    #[serde(default)]
    /// Mailbox defaults used by the host assembly.
    pub messaging: MessagingConfig,
    #[serde(default)]
    /// Caller wait budget for checked shutdown.
    pub shutdown: ShutdownConfig,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
/// Application identity in the composition schema.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct ApplicationConfig {
    /// Application-local name; nonempty when used for application identity.
    pub name: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
/// Explicit enable flag and owner-defined component configuration.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct ComponentSettings {
    #[serde(default)]
    /// Explicit opt-in; omitted values default to false.
    pub enabled: bool,
    #[serde(default)]
    /// Owner-defined TOML component settings converted to JSON.
    pub config: BTreeMap<String, toml::Value>,
}
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// Composition mailbox defaults; only reject is accepted by this TOML adapter.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct MessagingConfig {
    /// Per-component mailbox capacity; must be positive.
    pub default_queue_capacity: usize,
    /// Per-shard mailbox capacity; must be positive.
    pub per_shard_capacity: usize,
    /// Composition adapter policy; only "reject" is currently accepted.
    pub publish_when_full: String,
}
impl Default for MessagingConfig {
    fn default() -> Self {
        Self {
            default_queue_capacity: 1024,
            per_shard_capacity: 64,
            publish_when_full: "reject".into(),
        }
    }
}
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// Caller wait budget for checked shutdown.
///
/// Development API for modorch 0.0.3. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub struct ShutdownConfig {
    /// Positive caller shutdown wait limit in seconds.
    pub grace_period_secs: u64,
}
impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            grace_period_secs: 30,
        }
    }
}
impl CompositionConfig {
    /// Parse strict TOML and reject unknown or invalid composition settings.
    ///
    /// Contract: DEP-001. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    pub fn parse(input: &str) -> FrameworkResult<Self> {
        let config: Self =
            toml::from_str(input).map_err(|e| FrameworkError::InvalidConfig(e.to_string()))?;
        if config.application.name.trim().is_empty()
            || config.messaging.default_queue_capacity == 0
            || config.messaging.per_shard_capacity == 0
            || config.messaging.publish_when_full != "reject"
            || config.shutdown.grace_period_secs == 0
        {
            return Err(FrameworkError::InvalidConfig(
                "invalid composition settings; publish_when_full must be reject".into(),
            ));
        }
        Ok(config)
    }
    /// Convert enabled TOML entries into an explicit ComponentSelection.
    ///
    /// Contract: DEP-001. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    pub fn selection(&self) -> FrameworkResult<ComponentSelection> {
        let mut selection = ComponentSelection::new();
        for (kind, components) in [
            (ComponentKind::Initiator, &self.initiators),
            (ComponentKind::Module, &self.modules),
        ] {
            for (id, settings) in components {
                if settings.enabled {
                    let config = serde_json::to_value(&settings.config)
                        .map_err(|e| FrameworkError::InvalidConfig(e.to_string()))?;
                    selection.configure(
                        ComponentRef {
                            kind,
                            id: id.clone(),
                        },
                        config,
                    );
                }
            }
        }
        Ok(selection)
    }
    /// Return the configured caller shutdown wait budget.
    ///
    /// Contract: DEP-001. Development API: modorch 0.0.3; see crate-level examples and ownership limits.
    pub fn grace_period(&self) -> Duration {
        Duration::from_secs(self.shutdown.grace_period_secs)
    }
}
