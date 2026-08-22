use crate::LinuxResult;
use crate::control::JsonLinePeer;
use crate::manifest::{
    CourtRole, ManifestFile, ManifestTransport, PolicyFile, parse_rights, validate,
};
use crate::protocol::{
    WireCap, WireDemoConfig, WireMessage, WireRights, WireRingInfo, WireStatus, WireTrace,
};
use crate::shm_ring::SharedRing;
use court_hosted::{CapId, CorridorTransport, Descriptor, HostedRoot, Rights};
use serde_json::to_writer;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

pub struct RootConfig {
    pub run_dir: PathBuf,
    pub manifest: ManifestFile,
    pub policy: PolicyFile,
}

struct RootState {
    root: HostedRoot,
    court_ids: HashMap<String, court_hosted::CourtId>,
    descriptors: HashMap<String, Descriptor>,
    caps_by_id: HashMap<u64, CapId>,
    caps_by_key: HashMap<(String, String), (CapId, Rights)>,
    court_roles: HashMap<String, CourtRole>,
    policy: PolicyFile,
    ring_info: HashMap<String, WireRingInfo>,
    demo: WireDemoConfig,
    trace: TraceSink,
    _rings: Vec<SharedRing>,
}

struct TraceSink {
    file: File,
}

impl TraceSink {
    fn new(path: &Path) -> LinuxResult<Self> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)?;
        Ok(Self { file })
    }

    fn record(&mut self, event: WireTrace) -> LinuxResult<()> {
        to_writer(&mut self.file, &event)?;
        self.file.write_all(b"\n")?;
        self.file.flush()?;
        Ok(())
    }
}

pub fn run_packet_rx_demo(run_dir: PathBuf) -> LinuxResult<()> {
    run_manifest_demo(RootConfig {
        run_dir,
        manifest: ManifestFile::builtin_packet_rx(),
        policy: PolicyFile::builtin_packet_rx(),
    })
}

pub fn run_manifest_demo(config: RootConfig) -> LinuxResult<()> {
    validate(&config.manifest, &config.policy)?;
    std::fs::create_dir_all(&config.run_dir)?;
    let sock_path = config.run_dir.join("root.sock");
    let trace_path = config.run_dir.join("trace.ndjson");
    let _ = std::fs::remove_file(&sock_path);
    let listener = UnixListener::bind(&sock_path)?;

    let mut state = build_state(config, &trace_path)?;
    state.trace.record(
        WireTrace::new("lookup", "root", WireStatus::Ok).with_path(state.demo.path.clone()),
    )?;

    let mut children = spawn_courts(&state, &sock_path)?;
    let mut peers = accept_peers(&listener, children.len(), state.demo.clone())?;
    let app_name = find_app_court(&state)?;
    let app = peers
        .get_mut(&app_name)
        .ok_or_else(|| format!("{app_name} court did not connect to root"))?;
    handle_app_lookup_and_denied_open(&mut state, app, &app_name)?;

    for grant in state_policy_grants(&state)? {
        let peer = peers
            .get_mut(&grant.court)
            .ok_or_else(|| format!("court {} did not connect to root", grant.court))?;
        handle_grant_and_open(&mut state, peer, &grant.court, &grant.path, grant.rights)?;
    }

    let app = peers
        .get_mut(&app_name)
        .ok_or_else(|| format!("{app_name} court did not connect to root"))?;
    record_trace_from(&mut state, app, "app send")?;
    let recv_court = find_recv_grant_court(&state)?;
    let recv_peer = peers
        .get_mut(&recv_court)
        .ok_or_else(|| format!("{recv_court} court did not connect to root"))?;
    record_trace_from(&mut state, recv_peer, "receiver recv")?;

    run_revokes(&mut state, &mut peers)?;
    run_faults(&mut state, &mut peers, &mut children)?;
    run_peer_down(&mut state, &mut peers)?;

    for (court, child) in children.iter_mut() {
        if peers.contains_key(court) {
            expect_child_success(child, court)?;
        }
    }
    Ok(())
}

fn build_state(config: RootConfig, trace_path: &Path) -> LinuxResult<RootState> {
    let mut root = HostedRoot::new();
    let mut court_ids = HashMap::new();
    let mut court_roles = HashMap::new();
    for court in &config.manifest.courts {
        let court_id = root.create_court(&court.name);
        court_ids.insert(court.name.clone(), court_id);
        court_roles.insert(court.name.clone(), court.role);
    }

    let mut descriptors = HashMap::new();
    let mut ring_info = HashMap::new();
    let mut rings = Vec::new();
    for corridor in &config.manifest.corridors {
        let from = *court_ids
            .get(&corridor.from)
            .ok_or_else(|| format!("missing court {}", corridor.from))?;
        let to = *court_ids
            .get(&corridor.to)
            .ok_or_else(|| format!("missing court {}", corridor.to))?;
        let transport = match corridor.transport {
            ManifestTransport::SharedRing => CorridorTransport::SharedRing,
        };
        ck(root.create_corridor(
            &corridor.path,
            from,
            to,
            transport,
            corridor.capacity as usize,
        ))?;
        let descriptor = ck(root.lookup(from, &corridor.path))?;
        descriptors.insert(corridor.path.clone(), descriptor);

        let ring_path = config.run_dir.join(ring_file_name(&corridor.path));
        let ring = SharedRing::create(&ring_path, corridor.capacity, corridor.slot_size)?;
        ring_info.insert(
            corridor.path.clone(),
            WireRingInfo {
                path: ring_path.display().to_string(),
                capacity: corridor.capacity,
                slot_size: corridor.slot_size,
            },
        );
        rings.push(ring);
    }

    let demo_path = config
        .manifest
        .corridors
        .first()
        .ok_or("manifest must define at least one corridor")?
        .path
        .clone();
    Ok(RootState {
        root,
        court_ids,
        descriptors,
        caps_by_id: HashMap::new(),
        caps_by_key: HashMap::new(),
        court_roles,
        policy: config.policy,
        ring_info,
        demo: WireDemoConfig {
            path: demo_path,
            packet_payload: config.manifest.demo.packet_payload,
        },
        trace: TraceSink::new(trace_path)?,
        _rings: rings,
    })
}

fn spawn_courts(state: &RootState, sock_path: &Path) -> LinuxResult<HashMap<String, Child>> {
    let mut children = HashMap::new();
    for (court, _) in state.court_ids.iter() {
        let role = court_role(state, court)?;
        children.insert(court.clone(), spawn_court(role, court, sock_path)?);
    }
    Ok(children)
}

fn court_role(state: &RootState, court: &str) -> LinuxResult<&'static str> {
    state
        .court_roles
        .get(court)
        .map(|role| role.as_process_role())
        .ok_or_else(|| format!("unknown court {court}").into())
}

fn accept_peers(
    listener: &UnixListener,
    count: usize,
    demo: WireDemoConfig,
) -> LinuxResult<HashMap<String, JsonLinePeer>> {
    let mut peers = HashMap::new();
    while peers.len() < count {
        let (stream, _) = listener.accept()?;
        let mut peer = JsonLinePeer::new(stream)?;
        let court = match peer.recv()? {
            WireMessage::Hello { court } => court,
            other => return Err(format!("expected hello, got {other:?}").into()),
        };
        peer.send(&WireMessage::HelloAck {
            status: WireStatus::Ok,
            court: court.clone(),
            demo: demo.clone(),
        })?;
        peers.insert(court, peer);
    }
    Ok(peers)
}

#[derive(Clone)]
struct ResolvedGrant {
    court: String,
    path: String,
    rights: Rights,
}

fn state_policy_grants(state: &RootState) -> LinuxResult<Vec<ResolvedGrant>> {
    state
        .policy
        .grants
        .iter()
        .map(|grant| {
            Ok(ResolvedGrant {
                court: grant.court.clone(),
                path: grant.path.clone(),
                rights: parse_rights(&grant.rights)?,
            })
        })
        .collect()
}

fn handle_app_lookup_and_denied_open(
    state: &mut RootState,
    app: &mut JsonLinePeer,
    app_name: &str,
) -> LinuxResult<()> {
    match app.recv()? {
        WireMessage::Lookup { path } if path == state.demo.path => {
            let app_id = *state
                .court_ids
                .get(app_name)
                .ok_or_else(|| format!("unknown app court {app_name}"))?;
            let descriptor = ck(state.root.lookup(app_id, &path))?;
            state
                .trace
                .record(WireTrace::new("lookup", app_name, WireStatus::Ok).with_path(path))?;
            app.send(&WireMessage::LookupResult {
                status: WireStatus::Ok,
                descriptor: Some(crate::protocol::WireDescriptor::from_descriptor(
                    &descriptor,
                )),
            })?;
        }
        other => return Err(format!("expected app lookup, got {other:?}").into()),
    }

    match app.recv()? {
        WireMessage::Open {
            path,
            cap: None,
            rights,
        } if path == state.demo.path && rights.to_rights().contains(Rights::SEND) => {
            let app_id = *state
                .court_ids
                .get(app_name)
                .ok_or_else(|| format!("unknown app court {app_name}"))?;
            let descriptor = state
                .descriptors
                .get(&path)
                .ok_or_else(|| format!("unknown descriptor {path}"))?;
            let status = match state.root.open(app_id, descriptor, None, Rights::SEND) {
                Ok(_) => WireStatus::Ok,
                Err(error) => error.into(),
            };
            state
                .trace
                .record(WireTrace::new("open_denied", app_name, status).with_path(path))?;
            app.send(&WireMessage::OpenResult {
                status,
                cap: None,
                ring: None,
            })?;
        }
        other => return Err(format!("expected app unauthorized open, got {other:?}").into()),
    }
    Ok(())
}

fn handle_grant_and_open(
    state: &mut RootState,
    peer: &mut JsonLinePeer,
    court: &str,
    path: &str,
    rights: Rights,
) -> LinuxResult<CapId> {
    let court_id = *state
        .court_ids
        .get(court)
        .ok_or_else(|| format!("unknown court {court}"))?;
    let descriptor = state
        .descriptors
        .get(path)
        .ok_or_else(|| format!("unknown descriptor {path}"))?;
    let cap = ck(state.root.grant_corridor_cap(court_id, descriptor, rights))?;
    state.caps_by_id.insert(cap.raw(), cap);
    state
        .caps_by_key
        .insert((court.to_string(), path.to_string()), (cap, rights));
    let wire = wire_cap(cap, rights);
    state.trace.record(
        WireTrace::new("grant", "root", WireStatus::Ok)
            .with_path(path)
            .with_detail(format!("{court}:{}", cap.raw())),
    )?;
    peer.send(&WireMessage::Grant {
        path: path.to_string(),
        cap: wire.clone(),
    })?;

    match peer.recv()? {
        WireMessage::Open {
            path: request_path,
            cap: Some(request_cap),
            rights: request_rights,
        } if request_path == path => {
            let Some(core_cap) = state.caps_by_id.get(&request_cap.id).copied() else {
                peer.send(&WireMessage::OpenResult {
                    status: WireStatus::BadCap,
                    cap: None,
                    ring: None,
                })?;
                return Err("client opened with unknown cap".into());
            };
            let status = match state.root.open(
                court_id,
                descriptor,
                Some(core_cap),
                request_rights.to_rights(),
            ) {
                Ok(_) => WireStatus::Ok,
                Err(error) => error.into(),
            };
            state
                .trace
                .record(WireTrace::new("open", court, status).with_path(path))?;
            peer.send(&WireMessage::OpenResult {
                status,
                cap: if status == WireStatus::Ok {
                    Some(request_cap)
                } else {
                    None
                },
                ring: if status == WireStatus::Ok {
                    Some(
                        state
                            .ring_info
                            .get(path)
                            .ok_or_else(|| format!("missing ring info for {path}"))?
                            .clone(),
                    )
                } else {
                    None
                },
            })?;
            if status != WireStatus::Ok {
                return Err(format!("{court} open failed: {status:?}").into());
            }
        }
        other => return Err(format!("expected {court} open, got {other:?}").into()),
    }
    Ok(cap)
}

fn run_revokes(
    state: &mut RootState,
    peers: &mut HashMap<String, JsonLinePeer>,
) -> LinuxResult<()> {
    for revoke in state.policy.revokes.clone() {
        let Some((cap, rights)) = state
            .caps_by_key
            .get(&(revoke.court.clone(), revoke.path.clone()))
            .copied()
        else {
            return Err(format!(
                "no cap for revoke court={} path={}",
                revoke.court, revoke.path
            )
            .into());
        };
        ck(state.root.revoke(cap))?;
        state.trace.record(
            WireTrace::new("revoke", "root", WireStatus::Ok).with_path(revoke.path.clone()),
        )?;
        let peer = peers
            .get_mut(&revoke.court)
            .ok_or_else(|| format!("court {} did not connect to root", revoke.court))?;
        peer.send(&WireMessage::Revoke {
            cap: wire_cap(cap, rights),
        })?;
        record_trace_from(state, peer, "revoke")?;
    }
    Ok(())
}

fn run_faults(
    state: &mut RootState,
    peers: &mut HashMap<String, JsonLinePeer>,
    children: &mut HashMap<String, Child>,
) -> LinuxResult<()> {
    for fault in state.policy.faults.clone() {
        let peer = peers
            .get_mut(&fault.court)
            .ok_or_else(|| format!("court {} did not connect to root", fault.court))?;
        peer.send(&WireMessage::Fault {
            reason: fault.reason,
        })?;
        record_trace_from(state, peer, "fault")?;
        let child = children
            .get_mut(&fault.court)
            .ok_or_else(|| format!("missing child for {}", fault.court))?;
        expect_child_exit(child, 42, &fault.court)?;
        peers.remove(&fault.court);
        let court_id = *state
            .court_ids
            .get(&fault.court)
            .ok_or_else(|| format!("unknown court {}", fault.court))?;
        ck(state.root.fault_court(court_id))?;
        state.trace.record(
            WireTrace::new("court_fault", "root", WireStatus::Fault)
                .with_path(state.demo.path.clone()),
        )?;
    }
    Ok(())
}

fn run_peer_down(
    state: &mut RootState,
    peers: &mut HashMap<String, JsonLinePeer>,
) -> LinuxResult<()> {
    for peer_down in state.policy.peer_down.clone() {
        let peer = peers
            .get_mut(&peer_down.court)
            .ok_or_else(|| format!("court {} did not connect to root", peer_down.court))?;
        peer.send(&WireMessage::PeerDown {
            path: peer_down.path.clone(),
        })?;
        state.trace.record(
            WireTrace::new("peer_down", "root", WireStatus::PeerDown)
                .with_path(peer_down.path.clone()),
        )?;
        record_trace_from(state, peer, "peer_down")?;
        match peer.recv()? {
            WireMessage::DemoDone {
                court,
                status: WireStatus::Ok,
            } if court == peer_down.court => {
                state
                    .trace
                    .record(WireTrace::new("demo_done", court, WireStatus::Ok))?;
            }
            other => return Err(format!("expected DemoDone, got {other:?}").into()),
        }
    }
    Ok(())
}

fn record_trace_from(
    state: &mut RootState,
    peer: &mut JsonLinePeer,
    label: &str,
) -> LinuxResult<()> {
    match peer.recv()? {
        WireMessage::Trace { event } => state.trace.record(event),
        other => Err(format!("expected {label} trace, got {other:?}").into()),
    }
}

fn find_app_court(state: &RootState) -> LinuxResult<String> {
    state
        .court_roles
        .iter()
        .find(|(_, role)| **role == CourtRole::App)
        .map(|(court, _)| court.clone())
        .ok_or_else(|| "manifest must include one app court for MVP-0C".into())
}

fn find_recv_grant_court(state: &RootState) -> LinuxResult<String> {
    state
        .caps_by_key
        .iter()
        .find(|(_, (_, rights))| rights.contains(Rights::RECV))
        .map(|((court, _), _)| court.clone())
        .ok_or_else(|| "policy must grant one recv cap".into())
}

fn wire_cap(cap: CapId, rights: Rights) -> WireCap {
    WireCap {
        id: cap.raw(),
        rights: WireRights::from_rights(rights),
    }
}

fn ck<T>(result: court_hosted::CkResult<T>) -> LinuxResult<T> {
    result.map_err(|error| format!("core object-model error: {error:?}").into())
}

fn spawn_court(role: &str, court: &str, sock_path: &Path) -> LinuxResult<Child> {
    let mut command = court_command(role)?;
    command
        .arg("--root-sock")
        .arg(sock_path)
        .arg("--court")
        .arg(court)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    Ok(command.spawn()?)
}

fn court_command(role: &str) -> LinuxResult<Command> {
    let env_var = match role {
        "app" => "CK_APP_BIN",
        "net" => "CK_NET_BIN",
        _ => return Err(format!("unknown court role {role}").into()),
    };
    if let Ok(path) = std::env::var(env_var) {
        return Ok(Command::new(path));
    }

    let current = std::env::current_exe()?;
    let bin_name = match role {
        "app" => "ck-app",
        "net" => "ck-net",
        _ => unreachable!(),
    };
    if let Some(dir) = current.parent() {
        let sibling = dir.join(bin_name);
        if sibling.exists() {
            return Ok(Command::new(sibling));
        }
    }

    let mut command = Command::new(current);
    command.arg("--court-role").arg(role);
    Ok(command)
}

fn expect_child_success(child: &mut Child, name: &str) -> LinuxResult<()> {
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{name} exited with {status}").into())
    }
}

fn expect_child_exit(child: &mut Child, expected: i32, name: &str) -> LinuxResult<()> {
    let status = child.wait()?;
    if status.code() == Some(expected) {
        Ok(())
    } else {
        Err(format!("{name} exited with {status}, expected {expected}").into())
    }
}

fn ring_file_name(path: &str) -> String {
    let parts = path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let stem = if parts.len() >= 2 {
        format!("{}-{}", parts[parts.len() - 2], parts[parts.len() - 1])
    } else {
        path.trim_matches('/').replace('/', "-")
    };
    format!("{stem}.ring")
}
