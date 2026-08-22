#[cfg(unix)]
fn main() {
    if let Err(error) = unix_main() {
        eprintln!("ck-root: {error}");
        std::process::exit(1);
    }
}

#[cfg(unix)]
fn unix_main() -> court_hosted_linux::LinuxResult<()> {
    use court_hosted_linux::app::{AppConfig, run as run_app};
    use court_hosted_linux::manifest::{ManifestFile, PolicyFile};
    use court_hosted_linux::net::{NetConfig, run as run_net};
    use court_hosted_linux::root::{RootConfig, run_manifest_demo, run_packet_rx_demo};
    let args: Vec<String> = std::env::args().collect();
    if let Some(role) = value_of(&args, "--court-role") {
        let root_sock = required_path(&args, "--root-sock")?;
        let court = required_value(&args, "--court")?;
        return match role.as_str() {
            "app" => run_app(AppConfig { root_sock, court }),
            "net" => run_net(NetConfig { root_sock, court }),
            other => Err(format!("unknown --court-role {other}").into()),
        };
    }

    let run_dir = required_path(&args, "--run-dir")?;
    if let (Some(manifest), Some(policy)) =
        (value_of(&args, "--manifest"), value_of(&args, "--policy"))
    {
        return run_manifest_demo(RootConfig {
            run_dir,
            manifest: ManifestFile::from_path(&std::path::PathBuf::from(manifest))?,
            policy: PolicyFile::from_path(&std::path::PathBuf::from(policy))?,
        });
    }

    let demo = required_value(&args, "--demo")?;
    if demo != "packet-rx" {
        return Err(format!("unsupported demo {demo}").into());
    }
    run_packet_rx_demo(run_dir)
}

#[cfg(unix)]
fn value_of(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair[0] == flag)
        .map(|pair| pair[1].clone())
}

#[cfg(unix)]
fn required_value(args: &[String], flag: &str) -> court_hosted_linux::LinuxResult<String> {
    value_of(args, flag).ok_or_else(|| format!("missing {flag}").into())
}

#[cfg(unix)]
fn required_path(
    args: &[String],
    flag: &str,
) -> court_hosted_linux::LinuxResult<std::path::PathBuf> {
    Ok(std::path::PathBuf::from(required_value(args, flag)?))
}

#[cfg(not(unix))]
fn main() {
    eprintln!("ck-root is only available on Linux/WSL2 Unix targets");
}
