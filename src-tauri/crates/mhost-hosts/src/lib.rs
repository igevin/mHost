pub mod formatter;
pub mod parser;
pub mod validator;

pub use parser::{ParseResult, Parser};
pub use validator::{is_valid_domain, looks_like_ip};
