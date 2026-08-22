use crate::LinuxResult;
use court_hosted::Rights;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

pub const SCHEMA_VERSION: u32 = 1;

/// Linear demo pipeline used by MVP-0C. Policy `after` values must name a
/// phase that occurs strictly before the action they attach to.
pub const PHASE_ORDER: &[&str] = &[
    "lookup",
    "open_denied",
    "grant",
    "open",
    "send",
    "recv",
    "revoke",
    "send_after_revoke",
    "fault",
    "court_fault",
    "peer_down",
    "demo_done",
];

const PACKET_RX_MANIFEST: &str = include_str!("../../../fixtures/packet-rx/manifest.json");
const PACKET_RX_POLICY: &str = include_str!("../../../fixtures/packet-rx/policy.json");

fn schema_version_v1() -> u32 {
    SCHEMA_VERSION
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFile {
    #[serde(default = "schema_version_v1")]
    pub version: u32,
    pub courts: Vec<CourtManifest>,
    pub corridors: Vec<CorridorManifest>,
    pub demo: DemoManifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CourtManifest {
    pub name: String,
    pub role: CourtRole,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CourtRole {
    App,
    Net,
}

impl CourtRole {
    pub fn as_process_role(self) -> &'static str {
        match self {
            Self::App => "app",
            Self::Net => "net",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorridorManifest {
    pub path: String,
    pub transport: ManifestTransport,
    pub from: String,
    pub to: String,
    pub capacity: u32,
    pub slot_size: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestTransport {
    SharedRing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DemoManifest {
    pub packet_payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyFile {
    #[serde(default = "schema_version_v1")]
    pub version: u32,
    pub grants: Vec<GrantPolicy>,
    pub revokes: Vec<RevokePolicy>,
    pub faults: Vec<FaultPolicy>,
    pub peer_down: Vec<PeerDownPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantPolicy {
    pub court: String,
    pub path: String,
    pub rights: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokePolicy {
    pub court: String,
    pub path: String,
    pub after: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultPolicy {
    pub court: String,
    pub after: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerDownPolicy {
    pub court: String,
    pub path: String,
    pub after: String,
}

impl ManifestFile {
    pub fn from_path(path: &Path) -> LinuxResult<Self> {
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
    }

    pub fn builtin_packet_rx() -> Self {
        serde_json::from_str(PACKET_RX_MANIFEST).expect("embedded packet-rx manifest must parse")
    }
}

impl PolicyFile {
    pub fn from_path(path: &Path) -> LinuxResult<Self> {
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
    }

    pub fn builtin_packet_rx() -> Self {
        serde_json::from_str(PACKET_RX_POLICY).expect("embedded packet-rx policy must parse")
    }
}

pub fn validate(manifest: &ManifestFile, policy: &PolicyFile) -> LinuxResult<()> {
    require_schema("manifest", manifest.version)?;
    require_schema("policy", policy.version)?;

    if manifest.demo.packet_payload.is_empty() {
        return Err("demo packet_payload must not be empty".into());
    }

    let mut courts = HashSet::new();
    let mut app_count = 0usize;
    let mut net_count = 0usize;
    for court in &manifest.courts {
        if court.name.is_empty() {
            return Err("court name must not be empty".into());
        }
        if !courts.insert(court.name.as_str()) {
            return Err(format!("duplicate court name {}", court.name).into());
        }
        match court.role {
            CourtRole::App => app_count += 1,
            CourtRole::Net => net_count += 1,
        }
    }
    if app_count != 1 {
        return Err(format!(
            "MVP-0C manifest must declare exactly one app court, found {app_count}"
        )
        .into());
    }
    if net_count < 1 {
        return Err("MVP-0C manifest must declare at least one net court".into());
    }
    if manifest.corridors.is_empty() {
        return Err("MVP-0C manifest must declare at least one corridor".into());
    }

    let mut paths = HashSet::new();
    for corridor in &manifest.corridors {
        if !corridor.path.starts_with("/court/") || corridor.path == "/court/" {
            return Err(format!(
                "corridor path {} must be under /court/<name>/...",
                corridor.path
            )
            .into());
        }
        if !paths.insert(corridor.path.as_str()) {
            return Err(format!("duplicate corridor path {}", corridor.path).into());
        }
        if corridor.from == corridor.to {
            return Err(format!("corridor {} endpoints must be distinct", corridor.path).into());
        }
        if !courts.contains(corridor.from.as_str()) {
            return Err(format!(
                "corridor {} references missing from court {}",
                corridor.path, corridor.from
            )
            .into());
        }
        if !courts.contains(corridor.to.as_str()) {
            return Err(format!(
                "corridor {} references missing to court {}",
                corridor.path, corridor.to
            )
            .into());
        }
        if corridor.capacity == 0 {
            return Err(format!("corridor {} capacity must be nonzero", corridor.path).into());
        }
        if corridor.slot_size == 0 {
            return Err(format!("corridor {} slot_size must be nonzero", corridor.path).into());
        }
    }

    if policy.grants.is_empty() {
        return Err("policy must declare at least one grant".into());
    }

    let mut grant_keys = HashSet::new();
    let mut has_send = false;
    let mut has_recv = false;
    for grant in &policy.grants {
        require_court(&courts, &grant.court)?;
        require_path(&paths, &grant.path)?;
        if grant.rights.is_empty() {
            return Err(format!(
                "grant for court {} path {} must declare rights",
                grant.court, grant.path
            )
            .into());
        }
        let rights = parse_rights(&grant.rights)?;
        if rights.contains(Rights::SEND) {
            has_send = true;
        }
        if rights.contains(Rights::RECV) {
            has_recv = true;
        }
        grant_keys.insert((grant.court.as_str(), grant.path.as_str()));
    }
    if !has_send {
        return Err("policy must grant send on at least one corridor".into());
    }
    if !has_recv {
        return Err("policy must grant recv on at least one corridor".into());
    }

    for revoke in &policy.revokes {
        require_court(&courts, &revoke.court)?;
        require_path(&paths, &revoke.path)?;
        if !grant_keys.contains(&(revoke.court.as_str(), revoke.path.as_str())) {
            return Err(format!(
                "revoke court={} path={} does not match a grant",
                revoke.court, revoke.path
            )
            .into());
        }
        require_phase_before(&revoke.after, "revoke")?;
    }
    for fault in &policy.faults {
        require_court(&courts, &fault.court)?;
        if fault.reason.is_empty() {
            return Err(format!("fault for court {} must include a reason", fault.court).into());
        }
        require_phase_before(&fault.after, "fault")?;
    }
    for peer_down in &policy.peer_down {
        require_court(&courts, &peer_down.court)?;
        require_path(&paths, &peer_down.path)?;
        require_phase_before(&peer_down.after, "peer_down")?;
    }

    Ok(())
}

pub fn parse_rights(values: &[String]) -> LinuxResult<Rights> {
    let mut rights = Rights::NONE;
    for value in values {
        rights |= match value.as_str() {
            "read" => Rights::READ,
            "write" => Rights::WRITE,
            "send" => Rights::SEND,
            "recv" => Rights::RECV,
            "delegate" => Rights::DELEGATE,
            "revoke" => Rights::REVOKE,
            "observe" => Rights::OBSERVE,
            other => return Err(format!("unknown right {other}").into()),
        };
    }
    Ok(rights)
}

pub fn phase_index(name: &str) -> Option<usize> {
    PHASE_ORDER.iter().position(|phase| *phase == name)
}

fn require_schema(kind: &str, version: u32) -> LinuxResult<()> {
    if version == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(format!("unsupported {kind} version {version}, expected {SCHEMA_VERSION}").into())
    }
}

fn require_phase_before(after: &str, action: &str) -> LinuxResult<()> {
    let after_idx =
        phase_index(after).ok_or_else(|| format!("unknown policy after phase '{after}'"))?;
    let action_idx =
        phase_index(action).ok_or_else(|| format!("unknown action phase '{action}'"))?;
    if after_idx >= action_idx {
        return Err(format!(
            "policy after '{after}' is not before '{action}' in the MVP-0C pipeline"
        )
        .into());
    }
    Ok(())
}

fn require_court(courts: &HashSet<&str>, court: &str) -> LinuxResult<()> {
    if courts.contains(court) {
        Ok(())
    } else {
        Err(format!("policy references unknown court {court}").into())
    }
}

fn require_path(paths: &HashSet<&str>, path: &str) -> LinuxResult<()> {
    if paths.contains(path) {
        Ok(())
    } else {
        Err(format!("policy references unknown corridor path {path}").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_pair() -> (ManifestFile, PolicyFile) {
        (
            ManifestFile::builtin_packet_rx(),
            PolicyFile::builtin_packet_rx(),
        )
    }

    #[test]
    fn embedded_packet_rx_fixture_validates() {
        let (manifest, policy) = ok_pair();
        validate(&manifest, &policy).unwrap();
        assert_eq!(manifest.version, SCHEMA_VERSION);
        assert_eq!(policy.version, SCHEMA_VERSION);
        assert_eq!(manifest.demo.packet_payload, "mvp0c-packet");
        assert_eq!(manifest.corridors[0].path, "/court/net0/packet/rx");
    }

    #[test]
    fn duplicate_court_name_fails() {
        let (mut manifest, policy) = ok_pair();
        manifest.courts.push(CourtManifest {
            name: "app".to_string(),
            role: CourtRole::App,
        });

        assert!(validate(&manifest, &policy).is_err());
    }

    #[test]
    fn missing_corridor_endpoint_fails() {
        let (mut manifest, policy) = ok_pair();
        manifest.corridors[0].to = "missing".to_string();

        assert!(validate(&manifest, &policy).is_err());
    }

    #[test]
    fn unknown_right_fails() {
        let (manifest, mut policy) = ok_pair();
        policy.grants[0].rights.push("teleport".to_string());

        assert!(validate(&manifest, &policy).is_err());
    }

    #[test]
    fn unknown_policy_path_fails() {
        let (manifest, mut policy) = ok_pair();
        policy.grants[0].path = "/missing".to_string();

        assert!(validate(&manifest, &policy).is_err());
    }

    #[test]
    fn two_app_courts_fail() {
        let (mut manifest, policy) = ok_pair();
        manifest.courts.push(CourtManifest {
            name: "app2".to_string(),
            role: CourtRole::App,
        });
        let err = validate(&manifest, &policy).unwrap_err().to_string();
        assert!(err.contains("exactly one app court"), "{err}");
    }

    #[test]
    fn same_corridor_endpoints_fail() {
        let (mut manifest, policy) = ok_pair();
        manifest.corridors[0].to = "app".to_string();
        let err = validate(&manifest, &policy).unwrap_err().to_string();
        assert!(err.contains("distinct"), "{err}");
    }

    #[test]
    fn corridor_path_must_be_under_court() {
        let (mut manifest, policy) = ok_pair();
        manifest.corridors[0].path = "/net0/rx".to_string();
        let err = validate(&manifest, &policy).unwrap_err().to_string();
        assert!(err.contains("/court/"), "{err}");
    }

    #[test]
    fn missing_send_grant_fails() {
        let (manifest, mut policy) = ok_pair();
        policy.grants.retain(|grant| grant.court != "app");
        policy.revokes.clear();
        policy.peer_down.clear();
        let err = validate(&manifest, &policy).unwrap_err().to_string();
        assert!(err.contains("grant send"), "{err}");
    }

    #[test]
    fn revoke_without_matching_grant_fails() {
        let (manifest, mut policy) = ok_pair();
        policy.grants.retain(|grant| grant.court == "app");
        policy.grants[0].rights = vec![
            "send".to_string(),
            "recv".to_string(),
            "observe".to_string(),
        ];
        policy.revokes[0].court = "net0".to_string();
        let err = validate(&manifest, &policy).unwrap_err().to_string();
        assert!(err.contains("does not match a grant"), "{err}");
    }

    #[test]
    fn revoke_after_fault_is_rejected() {
        let (manifest, mut policy) = ok_pair();
        policy.revokes[0].after = "fault".to_string();
        let err = validate(&manifest, &policy).unwrap_err().to_string();
        assert!(err.contains("not before 'revoke'"), "{err}");
    }

    #[test]
    fn unknown_after_phase_fails() {
        let (manifest, mut policy) = ok_pair();
        policy.faults[0].after = "coffee".to_string();
        let err = validate(&manifest, &policy).unwrap_err().to_string();
        assert!(err.contains("unknown policy after phase"), "{err}");
    }

    #[test]
    fn unsupported_schema_version_fails() {
        let (mut manifest, policy) = ok_pair();
        manifest.version = 99;
        let err = validate(&manifest, &policy).unwrap_err().to_string();
        assert!(err.contains("unsupported manifest version 99"), "{err}");
    }

    #[test]
    fn empty_grant_rights_fail() {
        let (manifest, mut policy) = ok_pair();
        policy.grants[0].rights.clear();
        let err = validate(&manifest, &policy).unwrap_err().to_string();
        assert!(err.contains("must declare rights"), "{err}");
    }
}
