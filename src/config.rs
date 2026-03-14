use std::env;

#[derive(Debug)]
pub struct Config {
    pub username: Option<String>,
    pub password: Option<String>,
    pub xaddr: Option<String>,
}

impl Config {
    pub fn from_args_and_env() -> Result<Self, String> {
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

fn next_arg_value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("Missing value for {flag}\n\n{}", usage()))
}

pub fn usage() -> &'static str {
    "Usage: cargo run -- [--username USER] [--password PASS] [--xaddr DEVICE_SERVICE_URL]\n\
Environment variables are also supported: ONVIF_USERNAME, ONVIF_PASSWORD, ONVIF_XADDR\n\
Example: ONVIF_USERNAME=admin ONVIF_PASSWORD=secret cargo run -- --xaddr http://192.168.1.10:2020/onvif/device_service"
}
