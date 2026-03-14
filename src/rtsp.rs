use std::env;
use std::path::PathBuf;
use std::process::Command;

use crate::models::{ResolvedProfile, RtspProbeMetadata};

pub fn derive_rtsp_authority(profiles: &[ResolvedProfile], device_xaddr: &str) -> Option<String> {
    for profile in profiles {
        if let Some(authority) = profile.stream_uri.as_deref().and_then(authority_from_uri) {
            return Some(authority);
        }
    }

    authority_from_uri(device_xaddr).map(|host| {
        if host.contains(':') {
            host
        } else {
            format!("{host}:554")
        }
    })
}

pub fn print_known_rtsp_urls(authority: &str, username: &str, password: &str) {
    let Some(ffprobe) = find_ffprobe() else {
        println!("- PTZ lens HD: rtsp://<username>:<password>@{authority}/stream1");
        println!("- PTZ lens SD: rtsp://<username>:<password>@{authority}/stream2");
        println!("- fixed lens HD: rtsp://<username>:<password>@{authority}/stream1?channel=2");
        println!("- fixed lens SD: rtsp://<username>:<password>@{authority}/stream2?channel=2");
        println!("  ffprobe not found, so resolutions were not verified in this run.");
        return;
    };

    for (label, path) in [
        ("PTZ lens HD", "stream1"),
        ("PTZ lens SD", "stream2"),
        ("fixed lens HD", "stream1?channel=2"),
        ("fixed lens SD", "stream2?channel=2"),
    ] {
        let rtsp_url = format!("rtsp://{username}:{password}@{authority}/{path}");
        let display_url = format!("rtsp://<username>:<password>@{authority}/{path}");
        let metadata = probe_rtsp_metadata(&ffprobe, &rtsp_url);

        if metadata.has_video {
            match (metadata.width.as_deref(), metadata.height.as_deref()) {
                (Some(width), Some(height)) => {
                    println!("- {label}: {display_url} ({width}x{height})");
                }
                _ => {
                    println!("- {label}: {display_url} (video confirmed)");
                }
            }
        } else if let Some(error) = metadata.error {
            println!("- {label}: {display_url} ({error})");
        } else {
            println!("- {label}: {display_url} (no video detected)");
        }
    }
}

fn authority_from_uri(uri: &str) -> Option<String> {
    let after_scheme = uri.split_once("://")?.1;
    let authority = after_scheme.split('/').next()?;
    let authority = authority.rsplit('@').next()?;
    Some(authority.to_string())
}

fn find_ffprobe() -> Option<PathBuf> {
    [
        "/opt/homebrew/bin/ffprobe",
        "/usr/local/bin/ffprobe",
        "ffprobe",
    ]
    .into_iter()
    .find_map(|candidate| {
        let path = PathBuf::from(candidate);
        if path.is_absolute() {
            path.exists().then_some(path)
        } else {
            which_in_path(candidate)
        }
    })
}

fn which_in_path(binary_name: &str) -> Option<PathBuf> {
    let path_var = env::var_os("PATH")?;
    env::split_paths(&path_var)
        .map(|dir| dir.join(binary_name))
        .find(|path| path.exists())
}

fn probe_rtsp_metadata(ffprobe: &PathBuf, rtsp_url: &str) -> RtspProbeMetadata {
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-rtsp_transport",
            "tcp",
            "-rw_timeout",
            "3000000",
            "-show_entries",
            "stream=codec_type,width,height",
            "-of",
            "default=noprint_wrappers=1",
            rtsp_url,
        ])
        .output();

    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            let mut has_video = false;
            let mut width = None;
            let mut height = None;

            for line in stdout.lines() {
                if let Some(value) = line.strip_prefix("codec_type=") {
                    if value == "video" {
                        has_video = true;
                    }
                } else if let Some(value) = line.strip_prefix("width=") {
                    width = Some(value.to_string());
                } else if let Some(value) = line.strip_prefix("height=") {
                    height = Some(value.to_string());
                }
            }

            let combined = format!("{}{}", stdout, stderr);
            let error = if combined.contains("404") {
                Some("404 not found".to_string())
            } else if combined.contains("401") {
                Some("401 unauthorized".to_string())
            } else if combined.contains("406") {
                Some("406 not acceptable".to_string())
            } else if combined.trim().is_empty() {
                None
            } else {
                Some(first_line(&combined).to_string())
            };

            RtspProbeMetadata {
                has_video,
                width,
                height,
                error,
            }
        }
        Err(error) => RtspProbeMetadata {
            has_video: false,
            width: None,
            height: None,
            error: Some(format!("ffprobe failed: {error}")),
        },
    }
}

fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("unknown error")
}
