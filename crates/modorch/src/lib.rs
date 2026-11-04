//! Public Facade for application-scoped modular orchestration.
//!
//! Applications depend on this crate; the host owns Tokio, signals, configuration and
//! logging. Runtime/eventhub/scheduler are default features; config-toml and
//! observability are opt-in. See COMP-002, LIFE-001 and COMPAT-001.
//!
//! # Ownership and lifecycle
//! Factories are registered before explicit selection. Build validates without creating
//! instances. Shutdown must finish before the host drops its runtime. Keep ServiceLease
//! alive through the actual operation, including synchronous executor work.
//!
//! # Errors and timeouts
//! Command/Event success means admission, not business completion. A running request
//! can return Indeterminate on timeout; checked shutdown can return StopIncomplete
//! while its transaction continues. Inspect StartupError cleanup separately from source.
//!
//! # Examples
//!
//! ```
//! use modorch::prelude::*;
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
//! runtime.block_on(async {
//!     let handle = ApplicationBuilder::new("documented-app").build()?.start().await?;
//!     handle.shutdown_checked(ShutdownOptions::graceful()).await?;
//!     Ok::<_, Box<dyn std::error::Error>>(())
//! })?;
//! # Ok(())
//! # }
//! ```
//!
//! # Stability
//! Development API 0.0.2, Rust 1.85+. Match the actual source fingerprint when using
//! an Agent Skill. No Registry release or cross-process durability is implied.
#![forbid(unsafe_code)]
#![deny(missing_docs)]
pub use async_trait::async_trait;
#[cfg(feature = "config-toml")]
pub use modorch_config as config;
pub use modorch_core::*;
#[cfg(feature = "eventhub")]
pub use modorch_eventhub::*;
#[cfg(feature = "observability")]
pub use modorch_observe as observe;
pub use modorch_registry::*;
#[cfg(feature = "runtime")]
pub use modorch_runtime::{
    Application, ApplicationBuilder, ApplicationHandle, ComponentContext, ComponentHealth,
    ComponentState, DrainMetrics, Health, Initiator, InitiatorFactory, Lifecycle, LifecycleRecord,
    Module, ModuleContext, ModuleFactory, RuntimeContext, RuntimeMetrics, SetupContext,
    ShutdownContext, ShutdownOptions, ShutdownReceipt, StartupError,
};
#[cfg(feature = "scheduler")]
pub use modorch_scheduler::*;
/// Common Facade imports; feature-gated capabilities remain feature-gated.
///
/// Development API for modorch 0.0.2. Ownership, errors and runnable examples
/// are documented at crate level and in the matching behavior Contracts.
pub mod prelude {
    #[cfg(feature = "scheduler")]
    pub use crate::TaskScheduler;
    #[cfg(feature = "runtime")]
    pub use crate::{
        ApplicationBuilder, Initiator, InitiatorFactory, Lifecycle, Module, ModuleContext,
        ModuleFactory, RuntimeContext, SetupContext, ShutdownContext, ShutdownOptions,
    };
    pub use crate::{
        ComponentDescriptor, ComponentRef, ComponentSelection, FrameworkError, FrameworkResult,
        ServiceHandle, async_trait,
    };
    #[cfg(feature = "eventhub")]
    pub use crate::{
        EventHub, HandlerContext, HandlerRegistration, HandlerReply, MessageEnvelope, SerialHandler,
    };
}
