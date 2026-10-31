//! Modular orchestration as a library. The host owns its runtime and signals.
#![forbid(unsafe_code)]
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
