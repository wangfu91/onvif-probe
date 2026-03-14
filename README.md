# onvif-probe

`onvif-probe` is a small Rust CLI for discovering ONVIF cameras, enumerating their media service data, and surfacing RTSP URLs that are not always exposed cleanly through ONVIF.

This project is especially useful for dual-lens TP-Link/Tapo cameras where:

- ONVIF only exposes one lens
- the second lens is reachable over RTSP only
- the second lens uses a vendor-specific selector such as `?channel=2`

## What It Does

- Discovers ONVIF devices on the local network
- Prints advertised ONVIF service endpoints
- Queries media profiles and video sources
- Prints the RTSP URIs returned by ONVIF
- Probes known RTSP URLs with `ffprobe` and annotates them with detected resolutions

For compatible TP-Link/Tapo dual-lens cameras, it also prints the practical RTSP URLs that matter most:

- PTZ lens: `/stream1`, `/stream2`
- fixed lens: `/stream1?channel=2`, `/stream2?channel=2`

## Why This Exists

Some dual-lens cameras behave inconsistently across protocols:

- ONVIF may publish only the PTZ or wide-angle lens
- RTSP may expose additional lenses without advertising them through ONVIF
- vendor-specific URL shapes are often undocumented or model-dependent

This tool was built to close that gap and turn ad-hoc terminal probing into a repeatable workflow.

## Requirements

- Rust toolchain
- `ffprobe` in `PATH` for RTSP verification with resolution output

If `ffprobe` is not available, the tool still prints the candidate RTSP URLs, but it cannot verify resolution or stream presence.

## Build

```bash
cargo build
```

## Usage

Use environment variables:

```bash
ONVIF_USERNAME=admin \
ONVIF_PASSWORD=secret \
cargo run -- --xaddr http://192.168.1.10:2020/onvif/device_service
```

Or pass credentials as flags:

```bash
cargo run -- \
  --username admin \
  --password secret \
  --xaddr http://192.168.1.10:2020/onvif/device_service
```

If `--xaddr` is omitted, the tool uses the first ONVIF device service endpoint returned by discovery.

## Example Output

```text
Found 1 ONVIF device(s):
1. TP-IPC
   xaddrs: http://192.168.50.124:2020/onvif/device_service

Advertised ONVIF services:
- Media -> http://192.168.50.124:2020/onvif/service
- PTZ -> http://192.168.50.124:2020/onvif/service

Found 2 media profile(s):
- mainStream [profile_1] source=raw_vs1 -> rtsp://192.168.50.124:554/stream1
- minorStream [profile_2] source=raw_vs1 -> rtsp://192.168.50.124:554/stream2

Known RTSP URLs:
- PTZ lens HD: rtsp://<username>:<password>@192.168.50.124:554/stream1 (3840x2160)
- PTZ lens SD: rtsp://<username>:<password>@192.168.50.124:554/stream2 (640x360)
- fixed lens HD: rtsp://<username>:<password>@192.168.50.124:554/stream1?channel=2 (2560x1440)
- fixed lens SD: rtsp://<username>:<password>@192.168.50.124:554/stream2?channel=2 (320x240)
```

## TP-Link/Tapo Notes

For at least some dual-lens TP-Link/Tapo models, the effective mapping is:

- ONVIF media profiles: PTZ lens only
- RTSP PTZ lens: `/stream1`, `/stream2`
- RTSP fixed lens: `/stream1?channel=2`, `/stream2?channel=2`

This is not guaranteed across all firmware or models, but it is a strong starting point for affected devices.

## Limitations

- The underlying `onvif-rs` crate is old and limited; this project supplements it with direct SOAP calls.
- Camera-specific RTSP quirks vary by vendor and firmware.
- PTZ control for vendor-private lenses is outside the current scope.

## Safety Notes

- Do not commit real camera credentials.
- Prefer environment variables or your shell history controls when testing.

## Development

```bash
cargo fmt
cargo check
```
