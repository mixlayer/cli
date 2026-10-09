pub mod client;
pub mod commands;
pub mod config;

use url::Url;

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
