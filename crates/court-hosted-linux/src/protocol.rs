use court_hosted::{CkError, CorridorTransport, Descriptor, ObjectType, Rights};
use serde::{Deserialize, Serialize};

pub const PACKET_RX_PATH: &str = "/court/net0/packet/rx";
pub const DEFAULT_RING_CAPACITY: u32 = 64;
pub const DEFAULT_SLOT_SIZE: u32 = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireStatus {
    Ok,
    BadCap,
    NoRight,
    Revoked,
    NotFound,
    InvalidObject,
    InvalidState,
    PeerDown,
    QueueFull,
    QueueEmpty,
    Timeout,
    Fault,
}

impl From<CkError> for WireStatus {
    fn from(value: CkError) -> Self {
        match value {
            CkError::BadCap => Self::BadCap,
            CkError::NoRight => Self::NoRight,
            CkError::Revoked => Self::Revoked,
            CkError::NotFound => Self::NotFound,
            CkError::InvalidObject => Self::InvalidObject,
            CkError::InvalidState => Self::InvalidState,
            CkError::PeerDown => Self::PeerDown,
            CkError::QueueFull => Self::QueueFull,
            CkError::QueueEmpty => Self::QueueEmpty,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireRights {
    pub bits: u64,
}

impl WireRights {
    pub fn from_rights(rights: Rights) -> Self {
        Self { bits: rights.raw() }
    }

    pub fn to_rights(self) -> Rights {
        let mut rights = Rights::NONE;
        if self.bits & Rights::READ.raw() != 0 {
            rights |= Rights::READ;
        }
        if self.bits & Rights::WRITE.raw() != 0 {
            rights |= Rights::WRITE;
        }
        if self.bits & Rights::SEND.raw() != 0 {
            rights |= Rights::SEND;
        }
        if self.bits & Rights::RECV.raw() != 0 {
            rights |= Rights::RECV;
        }
        if self.bits & Rights::DELEGATE.raw() != 0 {
            rights |= Rights::DELEGATE;
        }
        if self.bits & Rights::REVOKE.raw() != 0 {
            rights |= Rights::REVOKE;
        }
        if self.bits & Rights::OBSERVE.raw() != 0 {
            rights |= Rights::OBSERVE;
        }
        rights
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireObjectType {
    Corridor,
}

impl From<ObjectType> for WireObjectType {
    fn from(value: ObjectType) -> Self {
        match value {
            ObjectType::Corridor => Self::Corridor,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireTransport {
    ControlChannel,
    SharedRing,
}

impl From<CorridorTransport> for WireTransport {
    fn from(value: CorridorTransport) -> Self {
        match value {
            CorridorTransport::ControlChannel => Self::ControlChannel,
            CorridorTransport::SharedRing => Self::SharedRing,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireDescriptor {
    pub path: String,
    pub object: u64,
    pub object_type: WireObjectType,
    pub transport: WireTransport,
}

impl WireDescriptor {
    pub fn from_descriptor(value: &Descriptor) -> Self {
        Self {
            path: value.path.clone(),
            object: value.object.raw(),
            object_type: value.object_type.into(),
            transport: value.transport.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireCap {
    pub id: u64,
    pub rights: WireRights,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireRingInfo {
    pub path: String,
    pub capacity: u32,
    pub slot_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireDemoConfig {
    pub path: String,
    pub packet_payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointState {
    pub cap: WireCap,
    pub revoked: bool,
    pub peer_down: bool,
}

impl EndpointState {
    pub fn new(cap: WireCap) -> Self {
        Self {
            cap,
            revoked: false,
            peer_down: false,
        }
    }

    pub fn mark_revoked(&mut self) {
        self.revoked = true;
    }

    pub fn mark_peer_down(&mut self) {
        self.peer_down = true;
    }

    pub fn status(&self) -> WireStatus {
        if self.peer_down {
            WireStatus::PeerDown
        } else if self.revoked {
            WireStatus::Revoked
        } else {
            WireStatus::Ok
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireTrace {
    pub event: String,
    pub court: String,
    pub path: Option<String>,
    pub status: WireStatus,
    pub len: Option<usize>,
    pub detail: Option<String>,
}

impl WireTrace {
    pub fn new(event: impl Into<String>, court: impl Into<String>, status: WireStatus) -> Self {
        Self {
            event: event.into(),
            court: court.into(),
            path: None,
            status,
            len: None,
            detail: None,
        }
    }

    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn with_len(mut self, len: usize) -> Self {
        self.len = Some(len);
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireMessage {
    Hello {
        court: String,
    },
    HelloAck {
        status: WireStatus,
        court: String,
        demo: WireDemoConfig,
    },
    Lookup {
        path: String,
    },
    LookupResult {
        status: WireStatus,
        descriptor: Option<WireDescriptor>,
    },
    Open {
        path: String,
        cap: Option<WireCap>,
        rights: WireRights,
    },
    OpenResult {
        status: WireStatus,
        cap: Option<WireCap>,
        ring: Option<WireRingInfo>,
    },
    Grant {
        path: String,
        cap: WireCap,
    },
    Revoke {
        cap: WireCap,
    },
    PeerDown {
        path: String,
    },
    Trace {
        event: WireTrace,
    },
    DemoDone {
        court: String,
        status: WireStatus,
    },
    Fault {
        reason: String,
    },
}
