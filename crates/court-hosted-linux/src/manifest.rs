use crate::LinuxResult;
use court_hosted::Rights;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFile {
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
        Self {
            courts: vec![
                CourtManifest {
                    name: "app".to_string(),
                    role: CourtRole::App,
                },
                CourtManifest {
                    name: "net0".to_string(),
                    role: CourtRole::Net,
                },
            ],
            corridors: vec![CorridorManifest {
                path: "/court/net0/packet/rx".to_string(),
                transport: ManifestTransport::SharedRing,
                from: "app".to_string(),
                to: "net0".to_string(),
                capacity: 64,
                slot_size: 2048,
            }],
            demo: DemoManifest {
                packet_payload: "mvp0b-packet".to_string(),
            },
        }
    }
}

impl PolicyFile {
    pub fn from_path(path: &Path) -> LinuxResult<Self> {
        Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
    }

    pub fn builtin_packet_rx() -> Self {
        Self {
            grants: vec![
                GrantPolicy {
                    court: "net0".to_string(),
                    path: "/court/net0/packet/rx".to_string(),
                    rights: vec!["recv".to_string(), "observe".to_string()],
                },
                GrantPolicy {
                    court: "app".to_string(),
                    path: "/court/net0/packet/rx".to_string(),
                    rights: vec!["send".to_string(), "observe".to_string()],
                },
            ],
            revokes: vec![RevokePolicy {
                court: "app".to_string(),
                path: "/court/net0/packet/rx".to_string(),
                after: "recv".to_string(),
            }],
            faults: vec![FaultPolicy {
                court: "net0".to_string(),
                after: "revoke".to_string(),
                reason: "mvp0b simulated net court crash".to_string(),
            }],
            peer_down: vec![PeerDownPolicy {
                court: "app".to_string(),
                path: "/court/net0/packet/rx".to_string(),
                after: "fault".to_string(),
            }],
        }
    }
}

pub fn validate(manifest: &ManifestFile, policy: &PolicyFile) -> LinuxResult<()> {
    let mut courts = HashSet::new();
    for court in &manifest.courts {
        if court.name.is_empty() {
            return Err("court name must not be empty".into());
        }
        if !courts.insert(court.name.as_str()) {
            return Err(format!("duplicate court name {}", court.name).into());
        }
    }

    let mut paths = HashSet::new();
    for corridor in &manifest.corridors {
        if !paths.insert(corridor.path.as_str()) {
            return Err(format!("duplicate corridor path {}", corridor.path).into());
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

    for grant in &policy.grants {
        require_court(&courts, &grant.court)?;
        require_path(&paths, &grant.path)?;
        parse_rights(&grant.rights)?;
    }
    for revoke in &policy.revokes {
        require_court(&courts, &revoke.court)?;
        require_path(&paths, &revoke.path)?;
    }
    for fault in &policy.faults {
        require_court(&courts, &fault.court)?;
    }
    for peer_down in &policy.peer_down {
        require_court(&courts, &peer_down.court)?;
        require_path(&paths, &peer_down.path)?;
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

    #[test]
    fn valid_manifest_and_policy_validate() {
        validate(
            &ManifestFile::builtin_packet_rx(),
            &PolicyFile::builtin_packet_rx(),
        )
        .unwrap();
    }

    #[test]
    fn duplicate_court_name_fails() {
        let mut manifest = ManifestFile::builtin_packet_rx();
        manifest.courts.push(CourtManifest {
            name: "app".to_string(),
            role: CourtRole::App,
        });

        assert!(validate(&manifest, &PolicyFile::builtin_packet_rx()).is_err());
    }

    #[test]
    fn missing_corridor_endpoint_fails() {
        let mut manifest = ManifestFile::builtin_packet_rx();
        manifest.corridors[0].to = "missing".to_string();

        assert!(validate(&manifest, &PolicyFile::builtin_packet_rx()).is_err());
    }

    #[test]
    fn unknown_right_fails() {
        let mut policy = PolicyFile::builtin_packet_rx();
        policy.grants[0].rights.push("teleport".to_string());

        assert!(validate(&ManifestFile::builtin_packet_rx(), &policy).is_err());
    }

    #[test]
    fn unknown_policy_path_fails() {
        let mut policy = PolicyFile::builtin_packet_rx();
        policy.grants[0].path = "/missing".to_string();

        assert!(validate(&ManifestFile::builtin_packet_rx(), &policy).is_err());
    }
}
