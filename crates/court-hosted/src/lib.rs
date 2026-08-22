//! Hosted Court Kernel object-model prototype.
//!
//! This crate is MVP-0A: an in-process model used to validate capability,
//! namespace, corridor, trace, revocation, and peer-fault semantics before the
//! project moves to a Linux multi-process prototype.

use std::collections::{HashMap, HashSet, VecDeque};
use std::error::Error;
use std::fmt;
use std::ops::{BitOr, BitOrAssign};

pub type CkResult<T> = Result<T, CkError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CkError {
    BadCap,
    NoRight,
    Revoked,
    NotFound,
    InvalidObject,
    InvalidState,
    PeerDown,
    QueueFull,
    QueueEmpty,
}

impl fmt::Display for CkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::BadCap => "bad capability",
            Self::NoRight => "missing right",
            Self::Revoked => "capability revoked",
            Self::NotFound => "not found",
            Self::InvalidObject => "invalid object",
            Self::InvalidState => "invalid state",
            Self::PeerDown => "peer down",
            Self::QueueFull => "queue full",
            Self::QueueEmpty => "queue empty",
        })
    }
}

impl Error for CkError {}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CourtId(u64);

impl CourtId {
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for CourtId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CourtId({})", self.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CapId(u64);

impl CapId {
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for CapId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CapId({})", self.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId(u64);

impl ObjectId {
    pub fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ObjectId({})", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectType {
    Corridor,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Rights(u64);

impl Rights {
    pub const NONE: Self = Self(0);
    pub const READ: Self = Self(1 << 0);
    pub const WRITE: Self = Self(1 << 1);
    pub const SEND: Self = Self(1 << 2);
    pub const RECV: Self = Self(1 << 3);
    pub const DELEGATE: Self = Self(1 << 4);
    pub const REVOKE: Self = Self(1 << 5);
    pub const OBSERVE: Self = Self(1 << 6);

    pub fn contains(self, required: Rights) -> bool {
        (self.0 & required.0) == required.0
    }

    pub fn raw(self) -> u64 {
        self.0
    }
}

impl fmt::Debug for Rights {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Rights(0x{:x})", self.0)
    }
}

impl BitOr for Rights {
    type Output = Rights;

    fn bitor(self, rhs: Rights) -> Self::Output {
        Rights(self.0 | rhs.0)
    }
}

impl BitOrAssign for Rights {
    fn bitor_assign(&mut self, rhs: Rights) {
        self.0 |= rhs.0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CourtState {
    Running,
    Faulted,
}

#[derive(Debug, Clone)]
pub struct Court {
    pub id: CourtId,
    pub name: String,
    pub state: CourtState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorridorTransport {
    ControlChannel,
    SharedRing,
}

#[derive(Debug, Clone)]
pub struct Descriptor {
    pub path: String,
    pub object: ObjectId,
    pub object_type: ObjectType,
    pub transport: CorridorTransport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceEvent {
    CourtCreated {
        court: CourtId,
        name: String,
    },
    CorridorCreated {
        path: String,
        object: ObjectId,
        transport: CorridorTransport,
    },
    NamespaceLookup {
        court: CourtId,
        path: String,
        found: bool,
    },
    CapGranted {
        court: CourtId,
        cap: CapId,
        object: ObjectId,
        rights: Rights,
    },
    OpenDenied {
        court: CourtId,
        path: String,
        reason: CkError,
    },
    Opened {
        court: CourtId,
        path: String,
        cap: CapId,
        rights: Rights,
    },
    Sent {
        court: CourtId,
        object: ObjectId,
        len: usize,
    },
    Received {
        court: CourtId,
        object: ObjectId,
        len: usize,
    },
    CapRevoked {
        cap: CapId,
    },
    CourtFaulted {
        court: CourtId,
    },
    PeerDown {
        court: CourtId,
        object: ObjectId,
    },
}

#[derive(Debug, Clone)]
struct Capability {
    object: ObjectId,
    object_type: ObjectType,
    rights: Rights,
    parent: Option<CapId>,
    revoked: bool,
    delegable: bool,
}

#[derive(Debug, Default)]
struct CapRegistry {
    next_cap: u64,
    caps: HashMap<CapId, Capability>,
    children: HashMap<CapId, Vec<CapId>>,
}

impl CapRegistry {
    fn mint_root(&mut self, object: ObjectId, object_type: ObjectType, rights: Rights) -> CapId {
        let cap = self.alloc_cap();
        self.caps.insert(
            cap,
            Capability {
                object,
                object_type,
                rights,
                parent: None,
                revoked: false,
                delegable: true,
            },
        );
        cap
    }

    fn delegate(&mut self, parent: CapId, rights: Rights) -> CkResult<CapId> {
        let parent_cap = self.caps.get(&parent).ok_or(CkError::BadCap)?;
        if parent_cap.revoked {
            return Err(CkError::Revoked);
        }
        if !parent_cap.delegable || !parent_cap.rights.contains(Rights::DELEGATE) {
            return Err(CkError::NoRight);
        }
        if !parent_cap.rights.contains(rights) {
            return Err(CkError::NoRight);
        }
        let object = parent_cap.object;
        let object_type = parent_cap.object_type;

        let cap = self.alloc_cap();
        self.caps.insert(
            cap,
            Capability {
                object,
                object_type,
                rights,
                parent: Some(parent),
                revoked: false,
                delegable: rights.contains(Rights::DELEGATE),
            },
        );
        self.children.entry(parent).or_default().push(cap);
        Ok(cap)
    }

    fn revoke(&mut self, cap: CapId) -> CkResult<()> {
        if !self.caps.contains_key(&cap) {
            return Err(CkError::BadCap);
        }

        let mut stack = vec![cap];
        while let Some(next) = stack.pop() {
            if let Some(capability) = self.caps.get_mut(&next) {
                capability.revoked = true;
            }
            if let Some(children) = self.children.get(&next).cloned() {
                stack.extend(children);
            }
        }

        Ok(())
    }

    fn authorize(&self, cap: CapId, object_type: ObjectType, rights: Rights) -> CkResult<ObjectId> {
        let capability = self.caps.get(&cap).ok_or(CkError::BadCap)?;
        if capability.revoked {
            return Err(CkError::Revoked);
        }
        if capability.object_type != object_type {
            return Err(CkError::InvalidObject);
        }
        if !capability.rights.contains(rights) {
            return Err(CkError::NoRight);
        }
        Ok(capability.object)
    }

    fn parent_of(&self, cap: CapId) -> Option<CapId> {
        self.caps.get(&cap).and_then(|capability| capability.parent)
    }

    fn alloc_cap(&mut self) -> CapId {
        self.next_cap += 1;
        CapId(self.next_cap)
    }
}

#[derive(Debug, Default)]
struct Namespace {
    entries: HashMap<String, Descriptor>,
}

impl Namespace {
    fn bind(&mut self, path: String, descriptor: Descriptor) {
        self.entries.insert(path, descriptor);
    }

    fn lookup(&self, path: &str) -> Option<Descriptor> {
        self.entries.get(path).cloned()
    }
}

#[derive(Debug)]
struct Corridor {
    endpoints: (CourtId, CourtId),
    queue: VecDeque<Vec<u8>>,
    capacity: usize,
}

impl Corridor {
    fn new(endpoints: (CourtId, CourtId), capacity: usize) -> Self {
        Self {
            endpoints,
            queue: VecDeque::new(),
            capacity,
        }
    }

    fn has_peer_down(&self, caller: CourtId, courts: &HashMap<CourtId, Court>) -> bool {
        [self.endpoints.0, self.endpoints.1]
            .into_iter()
            .filter(|court| *court != caller)
            .any(|court| {
                courts
                    .get(&court)
                    .is_none_or(|court| court.state != CourtState::Running)
            })
    }
}

#[derive(Debug, Default)]
pub struct HostedRoot {
    next_court: u64,
    next_object: u64,
    courts: HashMap<CourtId, Court>,
    cspaces: HashMap<CourtId, HashSet<CapId>>,
    cap_registry: CapRegistry,
    namespace: Namespace,
    corridors: HashMap<ObjectId, Corridor>,
    root_caps: HashMap<ObjectId, CapId>,
    trace: Vec<TraceEvent>,
}

impl HostedRoot {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_court(&mut self, name: impl Into<String>) -> CourtId {
        self.next_court += 1;
        let court = CourtId(self.next_court);
        let name = name.into();
        self.courts.insert(
            court,
            Court {
                id: court,
                name: name.clone(),
                state: CourtState::Running,
            },
        );
        self.cspaces.insert(court, HashSet::new());
        self.trace.push(TraceEvent::CourtCreated { court, name });
        court
    }

    pub fn create_corridor(
        &mut self,
        path: impl Into<String>,
        first: CourtId,
        second: CourtId,
        transport: CorridorTransport,
        capacity: usize,
    ) -> CkResult<ObjectId> {
        self.require_running_court(first)?;
        self.require_running_court(second)?;
        self.next_object += 1;
        let object = ObjectId(self.next_object);
        let path = path.into();
        let corridor = Corridor::new((first, second), capacity);
        self.corridors.insert(object, corridor);

        let rights =
            Rights::SEND | Rights::RECV | Rights::OBSERVE | Rights::DELEGATE | Rights::REVOKE;
        let root_cap = self
            .cap_registry
            .mint_root(object, ObjectType::Corridor, rights);
        self.root_caps.insert(object, root_cap);
        self.namespace.bind(
            path.clone(),
            Descriptor {
                path: path.clone(),
                object,
                object_type: ObjectType::Corridor,
                transport,
            },
        );
        self.trace.push(TraceEvent::CorridorCreated {
            path,
            object,
            transport,
        });
        Ok(object)
    }

    pub fn lookup(&mut self, court: CourtId, path: &str) -> CkResult<Descriptor> {
        self.require_running_court(court)?;
        let descriptor = self.namespace.lookup(path);
        self.trace.push(TraceEvent::NamespaceLookup {
            court,
            path: path.to_string(),
            found: descriptor.is_some(),
        });
        descriptor.ok_or(CkError::NotFound)
    }

    pub fn grant_corridor_cap(
        &mut self,
        court: CourtId,
        descriptor: &Descriptor,
        rights: Rights,
    ) -> CkResult<CapId> {
        self.require_running_court(court)?;
        if descriptor.object_type != ObjectType::Corridor {
            return Err(CkError::InvalidObject);
        }
        let root_cap = *self
            .root_caps
            .get(&descriptor.object)
            .ok_or(CkError::InvalidObject)?;
        let cap = self.cap_registry.delegate(root_cap, rights)?;
        self.cspaces.entry(court).or_default().insert(cap);
        self.trace.push(TraceEvent::CapGranted {
            court,
            cap,
            object: descriptor.object,
            rights,
        });
        Ok(cap)
    }

    pub fn open(
        &mut self,
        court: CourtId,
        descriptor: &Descriptor,
        cap: Option<CapId>,
        requested_rights: Rights,
    ) -> CkResult<CapId> {
        self.require_running_court(court)?;
        let Some(cap) = cap else {
            self.trace.push(TraceEvent::OpenDenied {
                court,
                path: descriptor.path.clone(),
                reason: CkError::NoRight,
            });
            return Err(CkError::NoRight);
        };
        if !self.cspace_contains(court, cap) {
            self.trace.push(TraceEvent::OpenDenied {
                court,
                path: descriptor.path.clone(),
                reason: CkError::BadCap,
            });
            return Err(CkError::BadCap);
        }
        match self
            .cap_registry
            .authorize(cap, descriptor.object_type, requested_rights)
        {
            Ok(object) if object == descriptor.object => {
                self.trace.push(TraceEvent::Opened {
                    court,
                    path: descriptor.path.clone(),
                    cap,
                    rights: requested_rights,
                });
                Ok(cap)
            }
            Ok(_) => {
                self.trace.push(TraceEvent::OpenDenied {
                    court,
                    path: descriptor.path.clone(),
                    reason: CkError::InvalidObject,
                });
                Err(CkError::InvalidObject)
            }
            Err(reason) => {
                self.trace.push(TraceEvent::OpenDenied {
                    court,
                    path: descriptor.path.clone(),
                    reason,
                });
                Err(reason)
            }
        }
    }

    pub fn send(
        &mut self,
        court: CourtId,
        cap: CapId,
        payload: impl Into<Vec<u8>>,
    ) -> CkResult<()> {
        self.require_running_court(court)?;
        if !self.cspace_contains(court, cap) {
            return Err(CkError::BadCap);
        }
        let object = self
            .cap_registry
            .authorize(cap, ObjectType::Corridor, Rights::SEND)?;
        let corridor = self
            .corridors
            .get_mut(&object)
            .ok_or(CkError::InvalidObject)?;
        if corridor.has_peer_down(court, &self.courts) {
            self.trace.push(TraceEvent::PeerDown { court, object });
            return Err(CkError::PeerDown);
        }
        if corridor.queue.len() >= corridor.capacity {
            return Err(CkError::QueueFull);
        }
        let payload = payload.into();
        let len = payload.len();
        corridor.queue.push_back(payload);
        self.trace.push(TraceEvent::Sent { court, object, len });
        Ok(())
    }

    pub fn recv(&mut self, court: CourtId, cap: CapId) -> CkResult<Vec<u8>> {
        self.require_running_court(court)?;
        if !self.cspace_contains(court, cap) {
            return Err(CkError::BadCap);
        }
        let object = self
            .cap_registry
            .authorize(cap, ObjectType::Corridor, Rights::RECV)?;
        let corridor = self
            .corridors
            .get_mut(&object)
            .ok_or(CkError::InvalidObject)?;
        let payload = corridor.queue.pop_front().ok_or(CkError::QueueEmpty)?;
        let len = payload.len();
        self.trace.push(TraceEvent::Received { court, object, len });
        Ok(payload)
    }

    pub fn revoke(&mut self, cap: CapId) -> CkResult<()> {
        self.cap_registry.revoke(cap)?;
        self.trace.push(TraceEvent::CapRevoked { cap });
        Ok(())
    }

    pub fn fault_court(&mut self, court: CourtId) -> CkResult<()> {
        let court_state = self.courts.get_mut(&court).ok_or(CkError::InvalidState)?;
        court_state.state = CourtState::Faulted;
        self.trace.push(TraceEvent::CourtFaulted { court });
        Ok(())
    }

    pub fn court(&self, court: CourtId) -> Option<&Court> {
        self.courts.get(&court)
    }

    pub fn trace(&self) -> &[TraceEvent] {
        &self.trace
    }

    pub fn cap_parent(&self, cap: CapId) -> Option<CapId> {
        self.cap_registry.parent_of(cap)
    }

    fn require_running_court(&self, court: CourtId) -> CkResult<()> {
        let court = self.courts.get(&court).ok_or(CkError::InvalidState)?;
        if court.state != CourtState::Running {
            return Err(CkError::InvalidState);
        }
        Ok(())
    }

    fn cspace_contains(&self, court: CourtId, cap: CapId) -> bool {
        self.cspaces
            .get(&court)
            .is_some_and(|cspace| cspace.contains(&cap))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACKET_RX: &str = "/court/net0/packet/rx";

    fn root_with_app_and_net() -> (HostedRoot, CourtId, CourtId, Descriptor) {
        let mut root = HostedRoot::new();
        let app = root.create_court("app");
        let net = root.create_court("net");
        root.create_corridor(PACKET_RX, app, net, CorridorTransport::SharedRing, 4)
            .unwrap();
        let descriptor = root.lookup(app, PACKET_RX).unwrap();
        (root, app, net, descriptor)
    }

    #[test]
    fn lookup_without_cap_does_not_authorize_open() {
        let (mut root, app, _net, descriptor) = root_with_app_and_net();

        let result = root.open(app, &descriptor, None, Rights::SEND);

        assert_eq!(result, Err(CkError::NoRight));
        assert!(root.trace().iter().any(|event| {
            matches!(
                event,
                TraceEvent::OpenDenied {
                    court,
                    reason: CkError::NoRight,
                    ..
                } if *court == app
            )
        }));
    }

    #[test]
    fn granted_cap_allows_shared_ring_send_recv() {
        let (mut root, app, net, descriptor) = root_with_app_and_net();
        let app_cap = root
            .grant_corridor_cap(app, &descriptor, Rights::SEND | Rights::OBSERVE)
            .unwrap();
        let net_cap = root
            .grant_corridor_cap(net, &descriptor, Rights::RECV | Rights::OBSERVE)
            .unwrap();

        let send_handle = root
            .open(app, &descriptor, Some(app_cap), Rights::SEND)
            .unwrap();
        let recv_handle = root
            .open(net, &descriptor, Some(net_cap), Rights::RECV)
            .unwrap();
        root.send(app, send_handle, b"packet-1".to_vec()).unwrap();

        let packet = root.recv(net, recv_handle).unwrap();

        assert_eq!(packet, b"packet-1");
    }

    #[test]
    fn revoked_cap_fails_future_send() {
        let (mut root, app, _net, descriptor) = root_with_app_and_net();
        let app_cap = root
            .grant_corridor_cap(app, &descriptor, Rights::SEND | Rights::OBSERVE)
            .unwrap();
        let send_handle = root
            .open(app, &descriptor, Some(app_cap), Rights::SEND)
            .unwrap();

        root.revoke(app_cap).unwrap();
        let result = root.send(app, send_handle, b"packet-after-revoke".to_vec());

        assert_eq!(result, Err(CkError::Revoked));
    }

    #[test]
    fn peer_crash_is_contained_and_observable() {
        let (mut root, app, net, descriptor) = root_with_app_and_net();
        let app_cap = root
            .grant_corridor_cap(app, &descriptor, Rights::SEND | Rights::OBSERVE)
            .unwrap();
        let send_handle = root
            .open(app, &descriptor, Some(app_cap), Rights::SEND)
            .unwrap();

        root.fault_court(net).unwrap();
        let result = root.send(app, send_handle, b"packet-after-peer-fault".to_vec());

        assert_eq!(result, Err(CkError::PeerDown));
        assert_eq!(root.court(app).unwrap().state, CourtState::Running);
        assert_eq!(root.court(net).unwrap().state, CourtState::Faulted);
        assert!(root.trace().iter().any(|event| {
            matches!(
                event,
                TraceEvent::PeerDown {
                    court,
                    object,
                } if *court == app && *object == descriptor.object
            )
        }));
    }

    #[test]
    fn delegated_caps_keep_derivation_parent() {
        let (mut root, app, _net, descriptor) = root_with_app_and_net();
        let app_cap = root
            .grant_corridor_cap(app, &descriptor, Rights::SEND | Rights::OBSERVE)
            .unwrap();

        assert!(root.cap_parent(app_cap).is_some());
    }
}
