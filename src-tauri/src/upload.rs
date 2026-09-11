//! Sharing destinations.
//!
//! ImgBB is a direct API upload. YouTube deliberately is not: the Data API
//! charges 1600 quota units per upload against a 10,000/day project default,
//! and an unaudited OAuth client has every video it uploads forced to private.
//! Handing the file to the real upload page sidesteps both and costs the user
//! one paste.

use std::path::Path;

use serde::Serialize;

/// ImgBB rejects anything larger than this outright.
const IMGBB_MAX_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
pub struct ImgbbResult {
    pub url: String,
    pub display_url: String,
    pub delete_url: String,
}

pub async fn imgbb(api_key: &str, path: &Path) -> Result<ImgbbResult, String> {
    if api_key.trim().is_empty() {
        return Err(
            "Add your ImgBB API key in Settings first - it's free from imgbb.com/api.".into(),
        );
    }

    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| format!("could not read the file: {e}"))?;
    if meta.len() > IMGBB_MAX_BYTES {
        return Err(format!(
            "That image is {:.1} MB. ImgBB's limit is 32 MB - try JPEG screenshots in Settings.",
            meta.len() as f64 / 1_048_576.0
        ));
    }

    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("could not read the file: {e}"))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "screenshot.png".into());
    let mime = match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => "image/png",
        _ => "image/jpeg",
    };

    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(name)
        .mime_str(mime)
        .map_err(|e| e.to_string())?;
    let form = reqwest::multipart::Form::new().part("image", part);

    let client = reqwest::Client::builder()
        .user_agent(concat!("FiveMClip/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .post("https://api.imgbb.com/1/upload")
        .query(&[("key", api_key.trim())])
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("Upload failed: {e}"))?;

    let status = response.status();
    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("ImgBB sent something unexpected: {e}"))?;

    if !status.is_success() || !body["success"].as_bool().unwrap_or(false) {
        let message = body["error"]["message"]
            .as_str()
            .unwrap_or("ImgBB rejected the upload");
        // A bad key is by far the most common failure and the generic message
        // does not make that obvious.
        if status == reqwest::StatusCode::BAD_REQUEST {
            return Err(format!("{message} (check your ImgBB API key)"));
        }
        return Err(message.to_string());
    }

    let data = &body["data"];
    let url = data["url"].as_str().unwrap_or_default().to_string();
    if url.is_empty() {
        return Err("ImgBB did not return a link".into());
    }
    Ok(ImgbbResult {
        display_url: data["display_url"].as_str().unwrap_or(&url).to_string(),
        delete_url: data["delete_url"].as_str().unwrap_or_default().to_string(),
        url,
    })
}

pub const YOUTUBE_UPLOAD_URL: &str = "https://www.youtube.com/upload";

/// Discord, via an incoming webhook.
///
/// A webhook is a URL that posts to one channel. No bot, no OAuth, no account -
/// which is the only reason this fits an app with no server behind it.
///
/// The URL is a secret in exactly the way the ImgBB key is: anyone holding it
/// can post to that channel as this webhook. It is stored locally, shown as a
/// password field, and never goes anywhere but discord.com.
pub async fn discord(webhook: &str, path: &Path, message: &str) -> Result<(), String> {
    let webhook = webhook.trim();
    if webhook.is_empty() {
        return Err("Add your Discord webhook URL in Settings first.".into());
    }
    if !is_discord_webhook(webhook) {
        return Err(
            "That does not look like a Discord webhook URL. It should start with \
             https://discord.com/api/webhooks/."
                .into(),
        );
    }

    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("could not read the file: {e}"))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "clip.mp4".into());

    let part = reqwest::multipart::Part::bytes(bytes).file_name(name);
    let mut form = reqwest::multipart::Form::new().part("files[0]", part);
    if !message.trim().is_empty() {
        form = form.text("content", message.trim().to_string());
    }

    let client = reqwest::Client::builder()
        .user_agent(concat!("FiveMClip/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .post(webhook)
        .multipart(form)
        .send()
        .await
        .map_err(|e| format!("Could not reach Discord: {e}"))?;

    let status = response.status();
    if status.is_success() {
        return Ok(());
    }

    // Discord's own message is more useful than ours for most failures, but the
    // two people actually hit deserve plain language.
    let body = response.text().await.unwrap_or_default();
    Err(match status.as_u16() {
        401 | 403 | 404 => "That webhook does not exist any more. Make a new one in the \
                            channel's settings and paste it in again."
            .to_string(),
        413 => "Discord rejected the clip for being too big. Lower the upload limit in \
                Settings to match your server, or trim it shorter."
            .to_string(),
        429 => "Discord is rate limiting you. Wait a moment and try again.".to_string(),
        _ => format!("Discord rejected the upload ({status}). {body}"),
    })
}

/// Deliberately strict. A webhook URL is a write credential for someone's
/// channel, and a typo that sends a clip to an attacker's server is not a
/// mistake worth being relaxed about.
pub fn is_discord_webhook(url: &str) -> bool {
    [
        "https://discord.com/api/webhooks/",
        "https://discordapp.com/api/webhooks/",
    ]
    .iter()
    .any(|prefix| url.starts_with(prefix))
}

#[cfg(test)]
mod discord_tests {
    use super::is_discord_webhook;

    #[test]
    fn accepts_the_real_thing() {
        assert!(is_discord_webhook(
            "https://discord.com/api/webhooks/123456/abcdef"
        ));
        // The old host still works and plenty of saved URLs use it.
        assert!(is_discord_webhook(
            "https://discordapp.com/api/webhooks/123456/abcdef"
        ));
    }

    #[test]
    fn refuses_anything_else() {
        for bad in [
            "",
            "discord.com/api/webhooks/1/2",
            "http://discord.com/api/webhooks/1/2",
            "https://discord.com.evil.example/api/webhooks/1/2",
            "https://evil.example/https://discord.com/api/webhooks/1/2",
            "https://discord.com/api/webhook/1/2",
        ] {
            assert!(!is_discord_webhook(bad), "{bad:?} should be refused");
        }
    }
}
