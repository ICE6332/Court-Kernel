use crate::LinuxResult;
use crate::control::JsonLinePeer;
use crate::protocol::{
    EndpointState, WireDemoConfig, WireMessage, WireRights, WireStatus, WireTrace,
};
use crate::shm_ring::SharedRing;
use court_hosted::Rights;
use std::path::PathBuf;

pub struct AppConfig {
    pub root_sock: PathBuf,
    pub court: String,
}

pub fn run(config: AppConfig) -> LinuxResult<()> {
    let mut peer = JsonLinePeer::connect_with_retry(&config.root_sock)?;
    peer.send(&WireMessage::Hello {
        court: config.court.clone(),
    })?;
    let demo = expect_hello_ack(&mut peer, &config.court)?;

    peer.send(&WireMessage::Lookup {
        path: demo.path.clone(),
    })?;
    let descriptor = match peer.recv()? {
        WireMessage::LookupResult {
            status: WireStatus::Ok,
            descriptor: Some(descriptor),
        } => descriptor,
        other => return Err(format!("unexpected lookup response: {other:?}").into()),
    };

    peer.send(&WireMessage::Open {
        path: descriptor.path.clone(),
        cap: None,
        rights: WireRights::from_rights(Rights::SEND),
    })?;
    match peer.recv()? {
        WireMessage::OpenResult {
            status: WireStatus::NoRight,
            ..
        } => {}
        other => return Err(format!("expected NO_RIGHT open response, got {other:?}").into()),
    }

    let cap = match peer.recv()? {
        WireMessage::Grant { path, cap } if path == demo.path => cap,
        other => return Err(format!("expected SEND grant, got {other:?}").into()),
    };
    peer.send(&WireMessage::Open {
        path: descriptor.path.clone(),
        cap: Some(cap.clone()),
        rights: WireRights::from_rights(Rights::SEND),
    })?;
    let ring_info = match peer.recv()? {
        WireMessage::OpenResult {
            status: WireStatus::Ok,
            ring: Some(ring),
            ..
        } => ring,
        other => return Err(format!("expected successful open, got {other:?}").into()),
    };

    let ring = SharedRing::open(&PathBuf::from(&ring_info.path))?;
    let mut endpoint = EndpointState::new(cap.clone());
    let payload = demo.packet_payload.as_bytes();
    ring.send(&endpoint, payload)
        .map_err(|status| format!("send failed: {status:?}"))?;
    peer.send(&WireMessage::Trace {
        event: WireTrace::new("send", &config.court, WireStatus::Ok)
            .with_path(demo.path.clone())
            .with_len(payload.len()),
    })?;

    loop {
        match peer.recv()? {
            WireMessage::Revoke { cap: revoked } if revoked.id == cap.id => {
                endpoint.mark_revoked();
                let status = match ring.send(&endpoint, b"after-revoke") {
                    Ok(()) => WireStatus::Ok,
                    Err(status) => status,
                };
                peer.send(&WireMessage::Trace {
                    event: WireTrace::new("send_after_revoke", &config.court, status)
                        .with_path(demo.path.clone()),
                })?;
            }
            WireMessage::PeerDown { path } if path == demo.path => {
                endpoint.mark_peer_down();
                peer.send(&WireMessage::Trace {
                    event: WireTrace::new("peer_down", &config.court, endpoint.status())
                        .with_path(demo.path),
                })?;
                peer.send(&WireMessage::DemoDone {
                    court: config.court,
                    status: WireStatus::Ok,
                })?;
                return Ok(());
            }
            other => return Err(format!("unexpected app message: {other:?}").into()),
        }
    }
}

fn expect_hello_ack(peer: &mut JsonLinePeer, court: &str) -> LinuxResult<WireDemoConfig> {
    match peer.recv()? {
        WireMessage::HelloAck {
            status: WireStatus::Ok,
            court: ack,
            demo,
        } if ack == court => Ok(demo),
        other => Err(format!("unexpected hello ack: {other:?}").into()),
    }
}
