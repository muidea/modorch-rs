//! Optional TOML composition adapter. Paths and environment merging belong to the host.
#![forbid(unsafe_code)]
use modorch_core::*;
use modorch_registry::ComponentSelection;
use serde::Deserialize;
use std::{collections::BTreeMap, time::Duration};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompositionConfig {
    pub application: ApplicationConfig,
    #[serde(default)]
    pub initiators: BTreeMap<String, ComponentSettings>,
    #[serde(default)]
    pub modules: BTreeMap<String, ComponentSettings>,
    #[serde(default)]
    pub messaging: MessagingConfig,
    #[serde(default)]
    pub shutdown: ShutdownConfig,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplicationConfig {
    pub name: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub config: BTreeMap<String, toml::Value>,
}
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MessagingConfig {
    pub default_queue_capacity: usize,
    pub per_shard_capacity: usize,
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
pub struct ShutdownConfig {
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
    pub fn grace_period(&self) -> Duration {
        Duration::from_secs(self.shutdown.grace_period_secs)
    }
}
