pub mod config;
pub mod platform;
pub mod resolver;
pub mod server;

pub use config::DnsConfig;
pub use resolver::RuleEngine;
pub use server::{DnsError, DnsServer};
