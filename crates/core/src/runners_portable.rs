//! Compatibility names for the shared HTTP contract on non-Linux platforms.
//! No Wine/Proton functionality is provided by this module.
pub const USER_AGENT: &str = "GameHandler";
#[derive(Debug)]
pub enum RunnerError {
    Http { message: String },
}
impl std::fmt::Display for RunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http { message } => f.write_str(message),
        }
    }
}
impl std::error::Error for RunnerError {}
pub mod proton {
    pub use crate::transport::{HttpClient, ResponseHead};
}
