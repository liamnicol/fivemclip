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
