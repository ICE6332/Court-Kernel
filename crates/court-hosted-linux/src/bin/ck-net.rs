#[cfg(unix)]
fn main() {
    if let Err(error) = unix_main() {
        eprintln!("ck-net: {error}");
        std::process::exit(1);
    }
}

#[cfg(unix)]
fn unix_main() -> court_hosted_linux::LinuxResult<()> {
    use court_hosted_linux::net::{NetConfig, run};
    use std::path::PathBuf;

    let args: Vec<String> = std::env::args().collect();
    run(NetConfig {
        root_sock: PathBuf::from(required_value(&args, "--root-sock")?),
        court: required_value(&args, "--court")?,
    })
}

#[cfg(unix)]
fn required_value(args: &[String], flag: &str) -> court_hosted_linux::LinuxResult<String> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].clone())
        .ok_or_else(|| format!("missing {flag}").into())
}

#[cfg(not(unix))]
fn main() {
    eprintln!("ck-net is only available on Linux/WSL2 Unix targets");
}
