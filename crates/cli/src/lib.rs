pub mod client;
mod command;
pub mod config_transfer;
mod interaction;
mod output;
pub mod parser;
mod policy;
mod product;
mod service;

pub use command::Command;
pub use interaction::{CliInteraction, TerminalInteraction};
pub use output::{
    AgentStatus, CommandError, CommandErrorKind, CommandResult, DoctorCheck, DoctorReport,
    DoctorStatus, DoctorSummary, ProductKind,
};
pub use policy::CommandPolicy;
pub use product::{ProductCommandAdapter, UnsupportedProductCommandAdapter};
pub use service::{CommandExecutor, CommandService};
