//! Minimal Root Court object model for bring-up.
//!
//! This is a no_std, no-alloc subset of the hosted `HostedRoot`: enough to
//! prove that capability + namespace exist after a real UEFI handoff. It is
//! not the RFC-0002 ABI and does not replace `court-hosted`.

const MAX_COURTS: usize = 8;
const MAX_CAPS: usize = 16;
const MAX_BINDINGS: usize = 16;

#[derive(Clone, Copy)]
struct Court {
    _id: u64,
    _name: &'static str,
}

#[derive(Clone, Copy)]
struct Cap {
    _id: u64,
    _object: u64,
    _rights: u64,
}

#[derive(Clone, Copy)]
struct Binding {
    path: &'static str,
    _object: u64,
}

pub struct RootCourt {
    next_court: u64,
    next_cap: u64,
    next_object: u64,
    courts: [Option<Court>; MAX_COURTS],
    caps: [Option<Cap>; MAX_CAPS],
    bindings: [Option<Binding>; MAX_BINDINGS],
}

impl RootCourt {
    pub const fn new() -> Self {
        Self {
            next_court: 0,
            next_cap: 0,
            next_object: 0,
            courts: [None; MAX_COURTS],
            caps: [None; MAX_CAPS],
            bindings: [None; MAX_BINDINGS],
        }
    }

    pub fn bootstrap(&mut self) -> Result<(), &'static str> {
        let root = self.create_court("root")?;
        let ns = self.alloc_object();
        self.bind("/court/root", ns)?;
        let _cap = self.mint(ns, 1 << 6)?; // OBSERVE
        let _ = root;
        Ok(())
    }

    pub fn court_count(&self) -> usize {
        self.courts.iter().filter(|slot| slot.is_some()).count()
    }

    pub fn cap_count(&self) -> usize {
        self.caps.iter().filter(|slot| slot.is_some()).count()
    }

    pub fn lookup(&self, path: &str) -> bool {
        self.bindings
            .iter()
            .flatten()
            .any(|binding| binding.path == path)
    }

    fn create_court(&mut self, name: &'static str) -> Result<u64, &'static str> {
        let slot = self
            .courts
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or("court table full")?;
        self.next_court += 1;
        let id = self.next_court;
        *slot = Some(Court {
            _id: id,
            _name: name,
        });
        Ok(id)
    }

    fn alloc_object(&mut self) -> u64 {
        self.next_object += 1;
        self.next_object
    }

    fn bind(&mut self, path: &'static str, object: u64) -> Result<(), &'static str> {
        let slot = self
            .bindings
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or("namespace full")?;
        *slot = Some(Binding {
            path,
            _object: object,
        });
        Ok(())
    }

    fn mint(&mut self, object: u64, rights: u64) -> Result<u64, &'static str> {
        let slot = self
            .caps
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or("cspace full")?;
        self.next_cap += 1;
        let id = self.next_cap;
        *slot = Some(Cap {
            _id: id,
            _object: object,
            _rights: rights,
        });
        Ok(id)
    }
}
