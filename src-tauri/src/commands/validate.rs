use mhost_hosts::{ParseErrorAtLine, ValidateResult};

#[tauri::command]
pub fn validate_hosts_text(text: String) -> ValidateResult {
    mhost_hosts::parser::Parser::parse_with_lines(&text)
}

#[tauri::command]
pub fn validate_hosts_errors(text: String) -> Vec<ParseErrorAtLine> {
    mhost_hosts::parser::Parser::parse_errors_only(&text)
}
