#[cfg(unix)]
const USAGE: &str = "\
Court Kernel hosted Root Court (MVP-0C)

Usage:
  ck-root --run-dir <dir> --demo packet-rx
  ck-root --run-dir <dir> --manifest <manifest.json> --policy <policy.json>
  ck-root -h | --help

`--demo packet-rx` runs the compiled-in copy of fixtures/packet-rx.
Manifest and policy must be supplied together.
";

fn main() {
    cfg_select! {
        unix => {
            if let Err(error) = unix_main() {
                eprintln!("ck-root: {error}");
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("ck-root is only available on Linux/WSL2 Unix targets");
        }
    }
}

#[cfg(unix)]
fn unix_main() -> court_hosted_linux::LinuxResult<()> {
    use court_hosted_linux::app::{run as run_app, AppConfig};
    use court_hosted_linux::manifest::{ManifestFile, PolicyFile};
    use court_hosted_linux::net::{run as run_net, NetConfig};
    use court_hosted_linux::root::{run_manifest_demo, run_packet_rx_demo, RootConfig};
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print!("{USAGE}");
        return Ok(());
    }

    if let Some(role) = value_of(&args, "--court-role") {
        let root_sock = required_path(&args, "--root-sock")?;
        let court = required_value(&args, "--court")?;
        return match role.as_str() {
            "app" => run_app(AppConfig { root_sock, court }),
            "net" => run_net(NetConfig { root_sock, court }),
            other => Err(format!("unknown --court-role {other}").into()),
        };
    }

    let manifest = value_of(&args, "--manifest");
    let policy = value_of(&args, "--policy");
    let demo = value_of(&args, "--demo");
    match (manifest, policy, demo) {
        (Some(manifest), Some(policy), None) => {
            let run_dir = required_path(&args, "--run-dir")?;
            run_manifest_demo(RootConfig {
                run_dir,
                manifest: ManifestFile::from_path(&std::path::PathBuf::from(manifest))?,
                policy: PolicyFile::from_path(&std::path::PathBuf::from(policy))?,
            })
        }
        (None, None, Some(demo)) => {
            if demo != "packet-rx" {
                return Err(format!("unsupported demo {demo}").into());
            }
            run_packet_rx_demo(required_path(&args, "--run-dir")?)
        }
        (Some(_), None, _) | (None, Some(_), _) => {
            Err("--manifest and --policy must be supplied together".into())
        }
        (Some(_), Some(_), Some(_)) => {
            Err("use either --demo or --manifest/--policy, not both".into())
        }
        (None, None, None) => {
            eprint!("{USAGE}");
            Err("missing --demo or --manifest/--policy".into())
        }
    }
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
