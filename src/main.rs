use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use rand::RngCore;
use reqwest::blocking::Client;
use roxmltree::Document;
use sha1::{Digest, Sha1};
use std::env;
use std::panic;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

const DEVICE_WSDL: &str = "http://www.onvif.org/ver10/device/wsdl";
const MEDIA_WSDL_CANDIDATES: [&str; 2] = [
    "http://www.onvif.org/ver10/media/wsdl",
    "http://www.onvif.org/ver20/media/wsdl",
];

#[derive(Debug)]
struct Config {
    username: Option<String>,
    password: Option<String>,
    xaddr: Option<String>,
}

#[derive(Debug)]
struct ServiceEndpoint {
    name: String,
    xaddr: String,
}

#[derive(Debug)]
struct MediaProfile {
    token: String,
    name: String,
    namespace: String,
    video_source_token: Option<String>,
}

#[derive(Debug)]
struct VideoSource {
    token: String,
    framerate: Option<String>,
    resolution: Option<(String, String)>,
}

#[derive(Debug)]
struct ResolvedProfile {
    stream_uri: Option<String>,
}

#[derive(Debug)]
struct RtspProbeMetadata {
    has_video: bool,
    width: Option<String>,
    height: Option<String>,
    error: Option<String>,
}

impl Config {
    fn from_args_and_env() -> Result<Self, String> {
        let mut username = env::var("ONVIF_USERNAME").ok();
        let mut password = env::var("ONVIF_PASSWORD").ok();
        let mut xaddr = env::var("ONVIF_XADDR").ok();

        let mut args = env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--username" => username = Some(next_arg_value(&mut args, "--username")?),
                "--password" => password = Some(next_arg_value(&mut args, "--password")?),
                "--xaddr" => xaddr = Some(next_arg_value(&mut args, "--xaddr")?),
                "--help" | "-h" => return Err(usage().to_string()),
                other => return Err(format!("Unknown argument: {other}\n\n{}", usage())),
            }
        }

        Ok(Self {
            username,
            password,
            xaddr,
        })
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = match Config::from_args_and_env() {
        Ok(config) => config,
        Err(message) => {
            println!("{message}");
            return Ok(());
        }
    };

    let probe_duration = Duration::from_secs(3);

    let devices = match panic::catch_unwind(|| onvif::start_probe(&probe_duration)) {
        Ok(result) => result?,
        Err(_) => {
            eprintln!(
                "ONVIF discovery did not complete successfully. Check that a device is reachable on the local network and try again."
            );
            return Ok(());
        }
    };

    if devices.is_empty() {
        println!("No ONVIF devices found.");
        return Ok(());
    }

    println!("Found {} ONVIF device(s):", devices.len());

    for (index, device) in devices.iter().enumerate() {
        println!("{}. {}", index + 1, device.name());
        println!("   urn: {}", device.urn());
        println!("   hardware: {}", device.hardware());
        println!("   location: {}", device.location());

        if device.types().is_empty() {
            println!("   types: <none>");
        } else {
            println!("   types: {}", device.types().join(", "));
        }

        if device.xaddrs().is_empty() {
            println!("   xaddrs: <none>");
        } else {
            println!("   xaddrs: {}", device.xaddrs().join(", "));
        }
    }

    let selected_xaddr = config.xaddr.clone().or_else(|| {
        devices
            .iter()
            .flat_map(|device| device.xaddrs().iter())
            .next()
            .cloned()
    });

    let Some(device_xaddr) = selected_xaddr else {
        println!("No ONVIF device service endpoint is available.");
        return Ok(());
    };

    let (Some(username), Some(password)) = (config.username.as_deref(), config.password.as_deref())
    else {
        println!("\nTo enumerate stream URIs, provide ONVIF credentials.");
        println!("Selected device service: {device_xaddr}");
        println!("Example:");
        println!(
            "  ONVIF_USERNAME=admin ONVIF_PASSWORD=secret cargo run -- --xaddr {device_xaddr}"
        );
        return Ok(());
    };

    let client = Client::builder().build()?;
    let capabilities_xml = get_capabilities_xml(&client, &device_xaddr, username, password)?;
    let service_endpoints = get_service_endpoints(&capabilities_xml)?;
    if service_endpoints.is_empty() {
        println!("\nNo ONVIF service endpoints were listed in GetCapabilities.");
    } else {
        println!("\nAdvertised ONVIF services:");
        for service in &service_endpoints {
            println!("- {} -> {}", service.name, service.xaddr);
        }
    }

    let media_service_uri = get_media_service_uri_from_capabilities(&capabilities_xml)?;
    println!("\nMedia service: {media_service_uri}");

    let video_sources = get_video_sources(&client, &media_service_uri, username, password)?;
    if video_sources.is_empty() {
        println!("No video sources were returned by the camera.");
    } else {
        println!("Found {} video source(s):", video_sources.len());
        for source in &video_sources {
            match (&source.resolution, &source.framerate) {
                (Some((width, height)), Some(framerate)) => {
                    println!(
                        "- source [{}] {}x{} @ {} fps",
                        source.token, width, height, framerate
                    );
                }
                (Some((width, height)), None) => {
                    println!("- source [{}] {}x{}", source.token, width, height);
                }
                _ => {
                    println!("- source [{}]", source.token);
                }
            }
        }
    }

    let profiles = get_profiles(&client, &media_service_uri, username, password)?;
    if profiles.is_empty() {
        println!("No media profiles were returned by the camera.");
        return Ok(());
    }

    println!("Found {} media profile(s):", profiles.len());
    let mut resolved_profiles = Vec::with_capacity(profiles.len());
    for profile in profiles {
        match get_stream_uri(&client, &media_service_uri, username, password, &profile) {
            Ok(stream_uri) => {
                if let Some(source_token) = &profile.video_source_token {
                    println!(
                        "- {} [{}] source={} -> {}",
                        profile.name, profile.token, source_token, stream_uri
                    );
                } else {
                    println!("- {} [{}] -> {}", profile.name, profile.token, stream_uri);
                }
                resolved_profiles.push(ResolvedProfile {
                    stream_uri: Some(stream_uri),
                });
            }
            Err(error) => {
                println!(
                    "- {} [{}] -> failed to fetch stream URI: {}",
                    profile.name, profile.token, error
                );
                resolved_profiles.push(ResolvedProfile { stream_uri: None });
            }
        }
    }

    if let Some(rtsp_authority) = derive_rtsp_authority(&resolved_profiles, &device_xaddr) {
        println!("\nKnown RTSP URLs:");
        print_known_rtsp_urls(&rtsp_authority, username, password);
    }

    Ok(())
}

fn next_arg_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("Missing value for {flag}\n\n{}", usage()))
}

fn usage() -> &'static str {
    "Usage: cargo run -- [--username USER] [--password PASS] [--xaddr DEVICE_SERVICE_URL]\n\
Environment variables are also supported: ONVIF_USERNAME, ONVIF_PASSWORD, ONVIF_XADDR\n\
Example: ONVIF_USERNAME=admin ONVIF_PASSWORD=secret cargo run -- --xaddr http://192.168.1.10:2020/onvif/device_service"
}

fn get_capabilities_xml(
    client: &Client,
    device_xaddr: &str,
    username: &str,
    password: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let body = format!(
        "<tds:GetCapabilities xmlns:tds=\"{DEVICE_WSDL}\"><tds:Category>All</tds:Category></tds:GetCapabilities>"
    );
    soap_request(
        client,
        device_xaddr,
        &format!("{DEVICE_WSDL}/GetCapabilities"),
        &body,
        username,
        password,
    )
}

fn get_service_endpoints(
    capabilities_xml: &str,
) -> Result<Vec<ServiceEndpoint>, Box<dyn std::error::Error>> {
    let doc = Document::parse(capabilities_xml)?;
    let mut services = Vec::new();

    for node in doc.descendants().filter(|node| node.is_element()) {
        if node.tag_name().name() != "XAddr" {
            continue;
        }

        let Some(xaddr) = node.text() else {
            continue;
        };
        let Some(parent) = node.parent_element() else {
            continue;
        };
        let service_name = parent.tag_name().name();
        if service_name == "Capabilities" || service_name == "CapabilitiesExtension" {
            continue;
        }

        services.push(ServiceEndpoint {
            name: service_name.to_string(),
            xaddr: xaddr.to_string(),
        });
    }

    Ok(services)
}

fn get_media_service_uri_from_capabilities(
    capabilities_xml: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let doc = Document::parse(capabilities_xml)?;

    for service_name in ["Media", "Media2"] {
        if let Some(uri) = doc
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == service_name)
            .and_then(|node| find_descendant_text(node, "XAddr"))
        {
            return Ok(uri);
        }
    }

    if let Some(uri) = doc
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "XAddr")
        .filter_map(|node| node.text())
        .find(|text| text.contains("media"))
    {
        return Ok(uri.to_string());
    }

    Err("Could not find a media service endpoint in GetCapabilities response".into())
}

fn derive_rtsp_authority(profiles: &[ResolvedProfile], device_xaddr: &str) -> Option<String> {
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

fn authority_from_uri(uri: &str) -> Option<String> {
    let after_scheme = uri.split_once("://")?.1;
    let authority = after_scheme.split('/').next()?;
    let authority = authority.rsplit('@').next()?;
    Some(authority.to_string())
}

fn print_known_rtsp_urls(authority: &str, username: &str, password: &str) {
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

fn get_profiles(
    client: &Client,
    media_xaddr: &str,
    username: &str,
    password: &str,
) -> Result<Vec<MediaProfile>, Box<dyn std::error::Error>> {
    for namespace in MEDIA_WSDL_CANDIDATES {
        let body = format!("<trt:GetProfiles xmlns:trt=\"{namespace}\"/>");
        let xml = soap_request(
            client,
            media_xaddr,
            &format!("{namespace}/GetProfiles"),
            &body,
            username,
            password,
        )?;
        let doc = Document::parse(&xml)?;
        let profiles = doc
            .descendants()
            .filter(|node| {
                node.is_element()
                    && (node.tag_name().name() == "Profiles"
                        || node.tag_name().name() == "Profiles2")
            })
            .filter_map(|node| {
                let token = node.attribute("token")?;
                let name = find_child_text(node, "Name").unwrap_or_else(|| token.to_string());
                let video_source_token = find_descendant_text(node, "SourceToken");
                Some(MediaProfile {
                    token: token.to_string(),
                    name,
                    namespace: namespace.to_string(),
                    video_source_token,
                })
            })
            .collect::<Vec<_>>();

        if !profiles.is_empty() {
            return Ok(profiles);
        }
    }

    Ok(Vec::new())
}

fn get_video_sources(
    client: &Client,
    media_xaddr: &str,
    username: &str,
    password: &str,
) -> Result<Vec<VideoSource>, Box<dyn std::error::Error>> {
    for namespace in MEDIA_WSDL_CANDIDATES {
        let body = format!("<trt:GetVideoSources xmlns:trt=\"{namespace}\"/>");
        let xml = soap_request(
            client,
            media_xaddr,
            &format!("{namespace}/GetVideoSources"),
            &body,
            username,
            password,
        )?;
        let doc = Document::parse(&xml)?;
        let sources = doc
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "VideoSources")
            .filter_map(|node| {
                let token = node.attribute("token")?;
                let width = find_descendant_text(node, "Width");
                let height = find_descendant_text(node, "Height");
                let resolution = match (width, height) {
                    (Some(width), Some(height)) => Some((width, height)),
                    _ => None,
                };
                let framerate = find_descendant_text(node, "Framerate");

                Some(VideoSource {
                    token: token.to_string(),
                    framerate,
                    resolution,
                })
            })
            .collect::<Vec<_>>();

        if !sources.is_empty() {
            return Ok(sources);
        }
    }

    Ok(Vec::new())
}

fn get_stream_uri(
    client: &Client,
    media_xaddr: &str,
    username: &str,
    password: &str,
    profile: &MediaProfile,
) -> Result<String, Box<dyn std::error::Error>> {
    let body = format!(
        "<trt:GetStreamUri xmlns:trt=\"{namespace}\" xmlns:tt=\"http://www.onvif.org/ver10/schema\">\
            <trt:StreamSetup>\
                <tt:Stream>RTP-Unicast</tt:Stream>\
                <tt:Transport><tt:Protocol>RTSP</tt:Protocol></tt:Transport>\
            </trt:StreamSetup>\
            <trt:ProfileToken>{token}</trt:ProfileToken>\
        </trt:GetStreamUri>",
        namespace = profile.namespace,
        token = xml_escape(&profile.token),
    );
    let xml = soap_request(
        client,
        media_xaddr,
        &format!("{}/GetStreamUri", profile.namespace),
        &body,
        username,
        password,
    )?;
    let doc = Document::parse(&xml)?;
    doc.descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "Uri")
        .and_then(|node| node.text())
        .map(str::to_string)
        .ok_or_else(|| "GetStreamUri response did not contain a URI".into())
}

fn soap_request(
    client: &Client,
    endpoint: &str,
    action: &str,
    body: &str,
    username: &str,
    password: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let envelope = build_soap_envelope(body, username, password);
    let response = client
        .post(endpoint)
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("application/soap+xml; charset=utf-8; action=\"{action}\""),
        )
        .header("SOAPAction", format!("\"{action}\""))
        .body(envelope)
        .send()?
        .error_for_status()?;

    Ok(response.text()?)
}

fn build_soap_envelope(body: &str, username: &str, password: &str) -> String {
    let mut nonce = [0_u8; 20];
    rand::thread_rng().fill_bytes(&mut nonce);

    let created = Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string();
    let mut sha1 = Sha1::new();
    sha1.update(nonce);
    sha1.update(created.as_bytes());
    sha1.update(password.as_bytes());

    let digest = BASE64.encode(sha1.finalize());
    let nonce_b64 = BASE64.encode(nonce);

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
        <soap:Envelope xmlns:soap=\"http://www.w3.org/2003/05/soap-envelope\"\
                       xmlns:wsse=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd\"\
                       xmlns:wsu=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd\">\
            <soap:Header>\
                <wsse:Security soap:mustUnderstand=\"1\">\
                    <wsse:UsernameToken>\
                        <wsse:Username>{username}</wsse:Username>\
                        <wsse:Password Type=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-username-token-profile-1.0#PasswordDigest\">{digest}</wsse:Password>\
                        <wsse:Nonce EncodingType=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-soap-message-security-1.0#Base64Binary\">{nonce_b64}</wsse:Nonce>\
                        <wsu:Created>{created}</wsu:Created>\
                    </wsse:UsernameToken>\
                </wsse:Security>\
            </soap:Header>\
            <soap:Body>{body}</soap:Body>\
        </soap:Envelope>",
        username = xml_escape(username),
        digest = digest,
        nonce_b64 = nonce_b64,
        created = created,
        body = body,
    )
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn find_child_text<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
) -> Option<String> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == local_name)
        .and_then(|child| child.text())
        .map(str::to_string)
}

fn find_descendant_text<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    local_name: &str,
) -> Option<String> {
    node.descendants()
        .find(|child| child.is_element() && child.tag_name().name() == local_name)
        .and_then(|child| child.text())
        .map(str::to_string)
}
