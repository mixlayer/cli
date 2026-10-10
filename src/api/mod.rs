pub mod client;
pub mod commands;
pub mod config;

use url::Url;

/// Platform API origin (organizations, keys, prices, the hosted MCP server):
/// `MIXLAYER_PLATFORM_URL` or `https://api.mixlayer.com`, without a trailing
/// slash. Rejects non-HTTP schemes and embedded credentials.
pub fn platform_base() -> anyhow::Result<String> {
    use crate::exit::{self, Code};
    let base = std::env::var("MIXLAYER_PLATFORM_URL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://api.mixlayer.com".into());
    let url = Url::parse(&base).map_err(|_| exit::err(Code::Config, "invalid MIXLAYER_PLATFORM_URL"))?;
    if !["http", "https"].contains(&url.scheme())
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || base
            .split("://")
            .nth(1)
            .is_some_and(|s| s.split('/').next().unwrap_or("").contains('@'))
    {
        return Err(exit::err(Code::Config, "invalid MIXLAYER_PLATFORM_URL"));
    }
    Ok(base.trim_end_matches('/').into())
}

/// All CLI-owned console links use the campaign of the command that printed them.
pub fn console_url(path: &str, command: &str) -> String {
    let mut url = Url::parse("https://console.mixlayer.com").expect("fixed console URL");
    url.set_path(path);
    url.query_pairs_mut()
        .append_pair("utm_source", "mixlayer-cli")
        .append_pair("utm_medium", "cli")
        .append_pair("utm_campaign", command);
    url.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attributed_links_preserve_destination_and_campaign() {
        assert_eq!(
            console_url("/sign-up", "auth-setup"),
            "https://console.mixlayer.com/sign-up?utm_source=mixlayer-cli&utm_medium=cli&utm_campaign=auth-setup"
        );
        assert!(console_url("/app/billing", "ask").contains("utm_campaign=ask"));
    }
}
