#[allow(dead_code)]
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[allow(dead_code)]
pub fn version_info() -> String {
    format!("rustdesk-cli {VERSION}")
}
