use crate::LinuxResult;
use crate::control::JsonLinePeer;
use crate::protocol::{
    EndpointState, WireDemoConfig, WireMessage, WireRights, WireStatus, WireTrace,
};
use crate::shm_ring::SharedRing;
use court_hosted::Rights;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

pub struct NetConfig {
    pub root_sock: PathBuf,
    pub court: String,
}

pub fn run(config: NetConfig) -> LinuxResult<()> {
    let mut peer = JsonLinePeer::connect_with_retry(&config.root_sock)?;
    peer.send(&WireMessage::Hello {
        court: config.court.clone(),
    })?;
    let demo = expect_hello_ack(&mut peer, &config.court)?;

    let cap = match peer.recv()? {
        WireMessage::Grant { path, cap } if path == demo.path => cap,
        other => return Err(format!("expected RECV grant, got {other:?}").into()),
    };
    peer.send(&WireMessage::Open {
        path: demo.path.clone(),
        cap: Some(cap.clone()),
        rights: WireRights::from_rights(Rights::RECV),
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
    let endpoint = EndpointState::new(cap);
    let deadline = Instant::now() + Duration::from_secs(5);
    let packet = loop {
        if Instant::now() > deadline {
            return Err("timed out waiting for packet".into());
        }
        match ring.recv(&endpoint) {
            Ok(Some(packet)) => break packet,
            Ok(None) => thread::sleep(Duration::from_millis(10)),
            Err(status) => return Err(format!("recv failed: {status:?}").into()),
        }
    };
    peer.send(&WireMessage::Trace {
        event: WireTrace::new("recv", &config.court, WireStatus::Ok)
            .with_path(demo.path.clone())
            .with_len(packet.len())
            .with_detail(String::from_utf8_lossy(&packet).into_owned()),
    })?;

    match peer.recv()? {
        WireMessage::Fault { reason } => {
            peer.send(&WireMessage::Trace {
                event: WireTrace::new("fault", &config.court, WireStatus::Fault)
                    .with_path(demo.path)
                    .with_detail(reason),
            })?;
            std::process::exit(42);
        }
        other => Err(format!("unexpected net message: {other:?}").into()),
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
