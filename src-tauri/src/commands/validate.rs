use mhost_hosts::ValidateResult;

#[tauri::command]
pub fn validate_hosts_text(text: String) -> ValidateResult {
    mhost_hosts::parser::Parser::parse_with_lines(&text)
}
