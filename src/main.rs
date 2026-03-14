mod config;
mod models;
mod onvif_client;
mod rtsp;

use config::Config;
use models::ResolvedProfile;
use reqwest::blocking::Client;
use std::error::Error;
use std::panic;
use std::time::Duration;

fn main() -> Result<(), Box<dyn Error>> {
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
    let capabilities_xml =
        onvif_client::get_capabilities_xml(&client, &device_xaddr, username, password)?;
    let service_endpoints = onvif_client::get_service_endpoints(&capabilities_xml)?;
    if service_endpoints.is_empty() {
        println!("\nNo ONVIF service endpoints were listed in GetCapabilities.");
    } else {
        println!("\nAdvertised ONVIF services:");
        for service in &service_endpoints {
            println!("- {} -> {}", service.name, service.xaddr);
        }
    }

    let media_service_uri =
        onvif_client::get_media_service_uri_from_capabilities(&capabilities_xml)?;
    println!("\nMedia service: {media_service_uri}");

    let video_sources =
        onvif_client::get_video_sources(&client, &media_service_uri, username, password)?;
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
                _ => println!("- source [{}]", source.token),
            }
        }
    }

    let profiles = onvif_client::get_profiles(&client, &media_service_uri, username, password)?;
    if profiles.is_empty() {
        println!("No media profiles were returned by the camera.");
        return Ok(());
    }

    println!("Found {} media profile(s):", profiles.len());
    let mut resolved_profiles = Vec::with_capacity(profiles.len());
    for profile in profiles {
        match onvif_client::get_stream_uri(
            &client,
            &media_service_uri,
            username,
            password,
            &profile,
        ) {
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

    if let Some(rtsp_authority) = rtsp::derive_rtsp_authority(&resolved_profiles, &device_xaddr) {
        println!("\nKnown RTSP URLs:");
        rtsp::print_known_rtsp_urls(&rtsp_authority, username, password);
    }

    Ok(())
}
