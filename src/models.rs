#[derive(Debug)]
pub struct ServiceEndpoint {
    pub name: String,
    pub xaddr: String,
}

#[derive(Debug)]
pub struct MediaProfile {
    pub token: String,
    pub name: String,
    pub namespace: String,
    pub video_source_token: Option<String>,
}

#[derive(Debug)]
pub struct VideoSource {
    pub token: String,
    pub framerate: Option<String>,
    pub resolution: Option<(String, String)>,
}

#[derive(Debug)]
pub struct ResolvedProfile {
    pub stream_uri: Option<String>,
}

#[derive(Debug)]
pub struct RtspProbeMetadata {
    pub has_video: bool,
    pub width: Option<String>,
    pub height: Option<String>,
    pub error: Option<String>,
}
