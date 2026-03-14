use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use reqwest::blocking::Client;
use roxmltree::Document;
use sha1::{Digest, Sha1};
use std::error::Error;

use crate::models::{MediaProfile, ServiceEndpoint, VideoSource};

const DEVICE_WSDL: &str = "http://www.onvif.org/ver10/device/wsdl";
const MEDIA_WSDL_CANDIDATES: [&str; 2] = [
    "http://www.onvif.org/ver10/media/wsdl",
    "http://www.onvif.org/ver20/media/wsdl",
];

pub fn get_capabilities_xml(
    client: &Client,
    device_xaddr: &str,
    username: &str,
    password: &str,
) -> Result<String, Box<dyn Error>> {
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

pub fn get_service_endpoints(
    capabilities_xml: &str,
) -> Result<Vec<ServiceEndpoint>, Box<dyn Error>> {
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

pub fn get_media_service_uri_from_capabilities(
    capabilities_xml: &str,
) -> Result<String, Box<dyn Error>> {
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

pub fn get_profiles(
    client: &Client,
    media_xaddr: &str,
    username: &str,
    password: &str,
) -> Result<Vec<MediaProfile>, Box<dyn Error>> {
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

pub fn get_video_sources(
    client: &Client,
    media_xaddr: &str,
    username: &str,
    password: &str,
) -> Result<Vec<VideoSource>, Box<dyn Error>> {
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

pub fn get_stream_uri(
    client: &Client,
    media_xaddr: &str,
    username: &str,
    password: &str,
    profile: &MediaProfile,
) -> Result<String, Box<dyn Error>> {
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
) -> Result<String, Box<dyn Error>> {
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
    let nonce: [u8; 20] = rand::random();

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
