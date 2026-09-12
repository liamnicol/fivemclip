//! Uploading to a bucket the user owns.
//!
//! Deliberately not a hosting service of ours. The app already asks for a
//! personal ImgBB key for the same reason: a shared credential inside a
//! distributed binary gets extracted, and running storage for other people
//! means accounts, billing, quotas and somebody else's video on your bill.
//! Pointing the app at the user's own S3-compatible bucket costs them a few
//! pence a month, paid to their provider, and costs us nothing to operate.
//!
//! Works against anything speaking S3 with SigV4 - Cloudflare R2, Backblaze B2,
//! MinIO, S3 itself. R2 is the one worth recommending, because it charges
//! nothing for egress and serving clips is almost entirely egress.
//!
//! The signing is done here rather than with an AWS SDK. It is one PUT, and the
//! canonical request for it is about a hundred lines; the SDK is several
//! hundred crates. `sign_tests` pins the result against signatures produced by
//! botocore, so this is checked against a real implementation rather than
//! against my reading of the specification.

use std::path::Path;

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

use fivemclip_capture::config::S3Target;

/// Payload hashing is skipped: over HTTPS, S3 accepts this in place of the
/// body's SHA-256. Hashing a two gigabyte session before uploading it would
/// mean reading the whole file twice for no security we do not already get from
/// TLS.
const UNSIGNED: &str = "UNSIGNED-PAYLOAD";

type HmacSha256 = Hmac<Sha256>;

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).expect("hmac takes any key length");
    mac.update(data.as_bytes());
    mac.finalize().into_bytes().to_vec()
}

fn sha256_hex(data: &str) -> String {
    hex::encode(Sha256::digest(data.as_bytes()))
}

/// Percent-encode one path segment the way SigV4 wants it.
///
/// Unreserved characters are left alone and everything else becomes uppercase
/// %XX. S3 encodes the path *once* - the generic SigV4 rule is to encode it
/// twice, and using that here produces a signature the server rejects. Pinned
/// by `sign_tests::a_key_needing_encoding_matches_botocore`.
fn encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Encode a key for use as a path, keeping the separators as separators.
pub fn encode_key(key: &str) -> String {
    key.split('/')
        .map(encode_segment)
        .collect::<Vec<_>>()
        .join("/")
}

/// The `Authorization` header for a PUT, and the headers it commits to.
///
/// `canonical_path` must already be encoded, and must be byte-identical to the
/// path the request is actually sent to - the signature covers it.
pub fn sign_put(
    target: &S3Target,
    canonical_path: &str,
    host: &str,
    content_type: &str,
    amz_date: &str,
) -> String {
    let date = &amz_date[..8];
    let scope = format!("{date}/{}/s3/aws4_request", target.region);

    // Sorted by header name, which is what the canonical form requires.
    let canonical_headers = format!(
        "content-type:{content_type}\nhost:{host}\nx-amz-content-sha256:{UNSIGNED}\nx-amz-date:{amz_date}\n"
    );
    let signed_headers = "content-type;host;x-amz-content-sha256;x-amz-date";

    let canonical_request =
        format!("PUT\n{canonical_path}\n\n{canonical_headers}\n{signed_headers}\n{UNSIGNED}");
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        sha256_hex(&canonical_request)
    );

    let key = hmac(format!("AWS4{}", target.secret_access_key).as_bytes(), date);
    let key = hmac(&key, &target.region);
    let key = hmac(&key, "s3");
    let key = hmac(&key, "aws4_request");
    let signature = hex::encode(hmac(&key, &string_to_sign));

    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        target.access_key_id
    )
}

/// Where a file ends up: the object key, without a leading slash.
///
/// Keeps the file's own name, under the user's prefix. The name already carries
/// a timestamp, which is what stops two clips colliding.
pub fn object_key(target: &S3Target, path: &Path) -> String {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "clip.mp4".into());
    let prefix = target.prefix.trim().trim_matches('/');
    if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    }
}

pub fn content_type_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("mp4") => "video/mp4",
        Some("mkv") => "video/x-matroska",
        Some("webm") => "video/webm",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        _ => "application/octet-stream",
    }
}

/// The link to hand out for an uploaded object.
///
/// A bucket's own endpoint is not usually publicly readable, so the user gives
/// the domain they have put in front of it. Without one, the endpoint URL is
/// returned - correct, and it will only work if they made the bucket public.
pub fn public_url(target: &S3Target, encoded_key: &str) -> String {
    let base = target.public_base.trim().trim_end_matches('/');
    if !base.is_empty() {
        return format!("{base}/{encoded_key}");
    }
    format!(
        "{}/{}/{encoded_key}",
        target.endpoint.trim().trim_end_matches('/'),
        target.bucket.trim()
    )
}

/// PUT a file into the bucket and return the link to share.
///
/// Streamed rather than read into memory: a session recording is measured in
/// gigabytes, and holding one in RAM to upload it would be a spike the rest of
/// the app has to survive.
pub async fn put(target: &S3Target, path: &Path) -> Result<String, String> {
    if !target.is_configured() {
        return Err("Set up your bucket in Settings first.".into());
    }

    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| format!("could not read the file: {e}"))?;

    let key = encode_key(&object_key(target, path));
    let endpoint = target.endpoint.trim().trim_end_matches('/');
    let url = format!("{endpoint}/{}/{key}", target.bucket.trim());
    // The signature covers the Host header, so it must match byte for byte what
    // the client will actually send - including the port, when there is one to
    // send. `Url::port()` is None for the scheme's default port, which is
    // exactly when HTTP leaves it out of the header. Signing the bare hostname
    // worked against R2 on 443 and failed against everything on a custom port,
    // which is most self-hosted buckets.
    let parsed =
        reqwest::Url::parse(&url).map_err(|e| format!("That endpoint is not a URL: {e}"))?;
    let name = parsed.host_str().ok_or("That endpoint has no host.")?;
    let host = match parsed.port() {
        Some(port) => format!("{name}:{port}"),
        None => name.to_string(),
    };

    let path_for_signing = format!("/{}/{key}", target.bucket.trim());
    let content_type = content_type_for(path);
    let amz_date = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let authorization = sign_put(target, &path_for_signing, &host, content_type, &amz_date);

    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| format!("could not open the file: {e}"))?;
    let body = reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(file));

    let client = reqwest::Client::builder()
        .user_agent(concat!("FiveMClip/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| e.to_string())?;

    let response = client
        .put(&url)
        .header("authorization", authorization)
        .header("x-amz-date", &amz_date)
        .header("x-amz-content-sha256", UNSIGNED)
        .header("content-type", content_type)
        .header("content-length", meta.len())
        .body(body)
        .send()
        .await
        .map_err(|e| format!("Upload failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(explain(status, &body));
    }
    Ok(public_url(target, &key))
}

/// Turn the bucket's answer into something worth reading.
///
/// S3 replies in XML with a code that says exactly what is wrong, and showing
/// "403 Forbidden" instead of "your key is wrong" costs somebody an evening.
fn explain(status: reqwest::StatusCode, body: &str) -> String {
    let code = body
        .split("<Code>")
        .nth(1)
        .and_then(|rest| rest.split("</Code>").next())
        .unwrap_or_default();
    match code {
        "SignatureDoesNotMatch" => {
            "The bucket rejected the signature. Check the secret access key.".into()
        }
        "InvalidAccessKeyId" => "That access key ID is not recognised by the bucket.".into(),
        "AccessDenied" => "Access denied. The key needs write permission on this bucket.".into(),
        "NoSuchBucket" => "There is no bucket by that name at this endpoint.".into(),
        "RequestTimeTooSkewed" => {
            "The bucket rejected the request because this PC's clock is wrong.".into()
        }
        "EntityTooLarge" => "That file is larger than the bucket will take in one upload.".into(),
        other if !other.is_empty() => format!("The bucket refused it: {other}."),
        _ => format!("The bucket refused it ({status})."),
    }
}

#[cfg(test)]
mod sign_tests {
    use super::*;

    fn target() -> S3Target {
        S3Target {
            endpoint: "https://abc123.r2.cloudflarestorage.com".into(),
            bucket: "clips".into(),
            region: "auto".into(),
            access_key_id: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            public_base: String::new(),
            prefix: String::new(),
        }
    }

    fn signature_of(header: &str) -> &str {
        header
            .split("Signature=")
            .nth(1)
            .expect("the header carries a signature")
    }

    /// Cross-checked against botocore's S3SigV4Auth for the same request.
    /// Getting this wrong means every upload is rejected with a signature
    /// mismatch, and no amount of reading the code tells you which byte.
    #[test]
    fn a_plain_key_matches_botocore() {
        let header = sign_put(
            &target(),
            "/clips/FiveMClip/Clip_2026-09-12_13-13-32.mp4",
            "abc123.r2.cloudflarestorage.com",
            "video/mp4",
            "20260912T131332Z",
        );
        assert_eq!(
            signature_of(&header),
            "3e340444ed7968f59ae404c15b8641c354bd8b11dfbe8d41a47195213619d7eb"
        );
    }

    /// The encode-once rule. Encoding the path twice - which is what the
    /// generic SigV4 rule says - gives a different signature and every upload
    /// of a file with a space in its name fails.
    #[test]
    fn a_key_needing_encoding_matches_botocore() {
        let header = sign_put(
            &target(),
            "/clips/a%20b/Clip%20%231.mp4",
            "abc123.r2.cloudflarestorage.com",
            "video/mp4",
            "20260912T131332Z",
        );
        assert_eq!(
            signature_of(&header),
            "d3665131729cd17deb0b2d69c7a31eacb5a0769a8eef7ac179b3e4ab30330f3f"
        );
    }

    /// A bucket on a non-default port signs the host *with* the port, because
    /// that is what the Host header carries. Signing the bare name passed
    /// against R2 on 443 and failed against every self-hosted bucket - caught
    /// end to end against a server that rechecked the signature, not by
    /// reading this code.
    #[test]
    fn a_host_with_a_port_matches_botocore() {
        let header = sign_put(
            &target(),
            "/clips/Clip.mp4",
            "127.0.0.1:9000",
            "video/mp4",
            "20260912T131332Z",
        );
        assert_eq!(
            signature_of(&header),
            "3e350ee0690cc49070526a8717cc3db1003ff7676fbd8afc7902c30afd0f6d84"
        );
    }

    #[test]
    fn the_credential_and_scope_are_in_the_header() {
        let header = sign_put(
            &target(),
            "/clips/x.mp4",
            "abc123.r2.cloudflarestorage.com",
            "video/mp4",
            "20260912T131332Z",
        );
        assert!(
            header.contains("Credential=AKIAIOSFODNN7EXAMPLE/20260912/auto/s3/aws4_request"),
            "{header}"
        );
        assert!(header.contains("SignedHeaders=content-type;host;x-amz-content-sha256;x-amz-date"));
    }

    #[test]
    fn keys_are_encoded_once_and_keep_their_separators() {
        assert_eq!(encode_key("FiveMClip/Clip_1.mp4"), "FiveMClip/Clip_1.mp4");
        assert_eq!(encode_key("a b/Clip #1.mp4"), "a%20b/Clip%20%231.mp4");
        // Already-safe characters must not be touched.
        assert_eq!(encode_key("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn the_prefix_shapes_the_key() {
        let mut t = target();
        assert_eq!(object_key(&t, Path::new("/c/Clip.mp4")), "Clip.mp4");
        t.prefix = "/fivem/clips/".into();
        assert_eq!(
            object_key(&t, Path::new("/c/Clip.mp4")),
            "fivem/clips/Clip.mp4"
        );
    }

    /// A custom domain is the normal case: a bucket endpoint is not usually
    /// readable by the people being sent the link.
    #[test]
    fn a_public_domain_is_preferred_for_the_link() {
        let mut t = target();
        assert_eq!(
            public_url(&t, "Clip.mp4"),
            "https://abc123.r2.cloudflarestorage.com/clips/Clip.mp4"
        );
        t.public_base = "https://clips.example.com/".into();
        assert_eq!(
            public_url(&t, "Clip.mp4"),
            "https://clips.example.com/Clip.mp4"
        );
    }
}

/// Exercises a real PUT against a real endpoint. Ignored unless the bucket is
/// named in the environment, so CI needs no credentials:
///
/// ```text
/// FIVEMCLIP_TEST_S3_ENDPOINT=https://<acct>.r2.cloudflarestorage.com \
/// FIVEMCLIP_TEST_S3_BUCKET=clips \
/// FIVEMCLIP_TEST_S3_KEY_ID=... FIVEMCLIP_TEST_S3_SECRET=... \
///   cargo test -p fivemclip live_upload -- --nocapture
/// ```
///
/// Worth running against your own bucket before trusting the settings screen:
/// it is the difference between "the signature is right" and "the upload
/// works", and those failed separately during development.
#[cfg(test)]
mod live_tests {
    use super::*;

    fn target_from_env() -> Option<S3Target> {
        Some(S3Target {
            endpoint: std::env::var("FIVEMCLIP_TEST_S3_ENDPOINT").ok()?,
            bucket: std::env::var("FIVEMCLIP_TEST_S3_BUCKET").ok()?,
            region: std::env::var("FIVEMCLIP_TEST_S3_REGION").unwrap_or_else(|_| "auto".into()),
            access_key_id: std::env::var("FIVEMCLIP_TEST_S3_KEY_ID").ok()?,
            secret_access_key: std::env::var("FIVEMCLIP_TEST_S3_SECRET").ok()?,
            public_base: std::env::var("FIVEMCLIP_TEST_S3_PUBLIC_BASE").unwrap_or_default(),
            prefix: std::env::var("FIVEMCLIP_TEST_S3_PREFIX").unwrap_or_default(),
        })
    }

    #[test]
    fn live_upload_is_accepted() {
        let Some(target) = target_from_env() else {
            eprintln!("skipped: set FIVEMCLIP_TEST_S3_ENDPOINT and friends");
            return;
        };

        let dir = std::env::temp_dir().join("fivemclip-s3-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A name with a space in it, because that is the case the encode-once
        // rule exists for and the one that silently failed first.
        let file = dir.join("Clip 2026-09-12 #1.mp4");
        std::fs::write(&file, vec![7u8; 64 * 1024]).unwrap();

        let url = tauri::async_runtime::block_on(put(&target, &file)).expect("upload succeeds");
        eprintln!("uploaded to {url}");
        assert!(url.ends_with("Clip%202026-09-12%20%231.mp4"), "{url}");
    }

    /// A wrong secret must come back as something a person can act on, not as
    /// a status code.
    #[test]
    fn live_upload_with_a_bad_secret_says_so() {
        let Some(mut target) = target_from_env() else {
            eprintln!("skipped: set FIVEMCLIP_TEST_S3_ENDPOINT and friends");
            return;
        };
        target.secret_access_key = "definitely-not-the-secret".into();

        let dir = std::env::temp_dir().join("fivemclip-s3-test-bad");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Clip.mp4");
        std::fs::write(&file, b"nope").unwrap();

        let err = tauri::async_runtime::block_on(put(&target, &file)).unwrap_err();
        eprintln!("error was: {err}");
        assert!(err.contains("secret access key"), "{err}");
    }
}
