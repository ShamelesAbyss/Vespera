//! Deterministic world state, independent of rendering and provider execution.

mod storage;

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};
use vespera_core::AgentId;

pub use storage::WorldStore;

const MAX_RESIDENTS: usize = 32;
const MAX_SITES: usize = 64;
const MAX_MEMORY: usize = 24;
const MAX_STOCK: u16 = 256;
const MAX_SHARED: u16 = 4096;
const MAX_CARRIED: u16 = 4;
const BUILD_COST: u16 = 6;
const MAX_SHELTERS: u16 = 256;

#[derive(Debug, thiserror::Error)]
pub enum WorldError {
    #[error("invalid world input: {0}")]
    Invalid(&'static str),
    #[error("world changed since this action was planned")]
    StaleRevision,
    #[error("resident was not found")]
    UnknownResident,
    #[error("world clock is exhausted")]
    ClockExhausted,
    #[error("world storage operation failed")]
    Storage,
    #[error("file is not a supported Vespera world database")]
    Format,
    #[error("world database already exists")]
    AlreadyExists,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub x: u16,
    pub z: u16,
}

impl Position {
    fn distance(self, target: Self) -> u32 {
        u32::from(self.x.abs_diff(target.x)) + u32::from(self.z.abs_diff(target.z))
    }

    fn toward(self, target: Self) -> Self {
        if self.x != target.x {
            Self {
                x: step(self.x, target.x),
                z: self.z,
            }
        } else {
            Self {
                x: self.x,
                z: step(self.z, target.z),
            }
        }
    }
}

fn step(current: u16, target: u16) -> u16 {
    match current.cmp(&target) {
        std::cmp::Ordering::Less => current + 1,
        std::cmp::Ordering::Greater => current - 1,
        std::cmp::Ordering::Equal => current,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Forager,
    Builder,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Goal {
    Gather,
    Deliver,
    Build,
    Wait,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResidentSpec {
    pub id: AgentId,
    pub name: String,
    pub role: Role,
    pub position: Position,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Site {
    pub position: Position,
    pub stock: u16,
    pub capacity: u16,
    pub regeneration: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldSpec {
    pub seed: u64,
    pub width: u16,
    pub depth: u16,
    pub depot: Position,
    pub residents: Vec<ResidentSpec>,
    pub sites: Vec<Site>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Moved { to: Position },
    Gathered { at: Position },
    Delivered { amount: u16 },
    Built {},
    Met { other: AgentId },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Memory {
    pub tick: u64,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resident {
    pub id: AgentId,
    pub name: String,
    pub role: Role,
    pub goal: Goal,
    pub position: Position,
    pub carried: u16,
    pub active: bool,
    pub relationships: BTreeMap<AgentId, u16>,
    memory: VecDeque<Memory>,
}

impl Resident {
    fn remember(&mut self, tick: u64, action: Action) {
        if self.memory.len() == MAX_MEMORY {
            self.memory.pop_front();
        }
        self.memory.push_back(Memory { tick, action });
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct World {
    seed: u64,
    width: u16,
    depth: u16,
    depot: Position,
    tick: u64,
    revision: u64,
    shared_stock: u16,
    shelters: u16,
    residents: Vec<Resident>,
    sites: Vec<Site>,
}

/// Renderer-facing state. Owned resident memories are omitted.
#[derive(Debug, Serialize)]
pub struct Snapshot<'a> {
    pub seed: u64,
    pub width: u16,
    pub depth: u16,
    pub depot: Position,
    pub tick: u64,
    pub revision: u64,
    pub shared_stock: u16,
    pub shelters: u16,
    pub residents: Vec<ResidentView<'a>>,
    pub sites: &'a [Site],
}

#[derive(Debug, Serialize)]
pub struct ResidentView<'a> {
    pub id: AgentId,
    pub name: &'a str,
    pub role: Role,
    pub goal: Goal,
    pub position: Position,
    pub carried: u16,
    pub active: bool,
    pub relationships: &'a BTreeMap<AgentId, u16>,
}

impl World {
    pub fn new(spec: WorldSpec) -> Result<Self, WorldError> {
        let world = Self {
            seed: spec.seed,
            width: spec.width,
            depth: spec.depth,
            depot: spec.depot,
            tick: 0,
            revision: 0,
            shared_stock: 0,
            shelters: 0,
            residents: spec
                .residents
                .into_iter()
                .map(|resident| Resident {
                    id: resident.id,
                    name: resident.name,
                    role: resident.role,
                    position: resident.position,
                    goal: Goal::Wait,
                    carried: 0,
                    active: true,
                    relationships: BTreeMap::new(),
                    memory: VecDeque::new(),
                })
                .collect(),
            sites: spec.sites,
        };
        world.validate()?;
        Ok(world)
    }

    pub fn snapshot(&self) -> Snapshot<'_> {
        Snapshot {
            seed: self.seed,
            width: self.width,
            depth: self.depth,
            depot: self.depot,
            tick: self.tick,
            revision: self.revision,
            shared_stock: self.shared_stock,
            shelters: self.shelters,
            residents: self
                .residents
                .iter()
                .map(|resident| ResidentView {
                    id: resident.id,
                    name: &resident.name,
                    role: resident.role,
                    goal: resident.goal,
                    position: resident.position,
                    carried: resident.carried,
                    active: resident.active,
                    relationships: &resident.relationships,
                })
                .collect(),
            sites: &self.sites,
        }
    }

    /// Read the specified owner's memories. The trusted host supplies the owner.
    pub fn memory(&self, owner: AgentId) -> Result<&VecDeque<Memory>, WorldError> {
        self.residents
            .iter()
            .find(|resident| resident.id == owner)
            .map(|resident| &resident.memory)
            .ok_or(WorldError::UnknownResident)
    }

    pub fn set_active(
        &mut self,
        resident: AgentId,
        active: bool,
        revision: u64,
    ) -> Result<(), WorldError> {
        let next = self.next_revision(revision)?;
        let resident = self
            .residents
            .iter_mut()
            .find(|entry| entry.id == resident)
            .ok_or(WorldError::UnknownResident)?;
        resident.active = active;
        if !active {
            resident.goal = Goal::Wait;
        }
        self.revision = next;
        Ok(())
    }

    /// Apply a one-cell movement to the observed world revision.
    pub fn move_resident(
        &mut self,
        id: AgentId,
        to: Position,
        revision: u64,
    ) -> Result<(), WorldError> {
        let next = self.next_revision(revision)?;
        if !self.contains(to) {
            return Err(WorldError::Invalid("movement bounds"));
        }
        let resident = self
            .residents
            .iter_mut()
            .find(|resident| resident.id == id)
            .ok_or(WorldError::UnknownResident)?;
        if !resident.active || resident.position.distance(to) != 1 {
            return Err(WorldError::Invalid(
                "movement requires an active resident and an adjacent cell",
            ));
        }
        resident.position = to;
        resident.remember(self.tick, Action::Moved { to });
        self.revision = next;
        Ok(())
    }

    pub fn advance(&mut self) -> Result<(), WorldError> {
        let next = self.next_revision(self.revision)?;
        let tick = self.tick.checked_add(1).ok_or(WorldError::ClockExhausted)?;
        self.tick = tick;
        self.revision = next;
        for site in &mut self.sites {
            site.stock = (site.stock + site.regeneration).min(site.capacity);
        }
        let count = self.residents.len();
        let start = (self.seed.wrapping_add(tick - 1) % count as u64) as usize;
        for offset in 0..count {
            self.act((start + offset) % count);
        }
        self.socialize();
        Ok(())
    }

    fn act(&mut self, index: usize) {
        let resident = &mut self.residents[index];
        if !resident.active {
            return;
        }
        let action = match resident.role {
            Role::Builder if self.shared_stock >= BUILD_COST && self.shelters < MAX_SHELTERS => {
                resident.goal = Goal::Build;
                if resident.position == self.depot {
                    self.shared_stock -= BUILD_COST;
                    self.shelters += 1;
                    Some(Action::Built {})
                } else {
                    Some(walk(resident, self.depot))
                }
            }
            Role::Builder => {
                resident.goal = Goal::Wait;
                None
            }
            Role::Forager if resident.carried == MAX_CARRIED => {
                deliver(resident, self.depot, &mut self.shared_stock)
            }
            Role::Forager => {
                let position = resident.position;
                let target = self
                    .sites
                    .iter_mut()
                    .filter(|site| site.stock > 0)
                    .min_by_key(|site| (position.distance(site.position), site.position));
                if let Some(site) = target {
                    resident.goal = Goal::Gather;
                    if position == site.position {
                        site.stock -= 1;
                        resident.carried += 1;
                        Some(Action::Gathered { at: site.position })
                    } else {
                        Some(walk(resident, site.position))
                    }
                } else if resident.carried > 0 {
                    deliver(resident, self.depot, &mut self.shared_stock)
                } else {
                    resident.goal = Goal::Wait;
                    None
                }
            }
        };
        if let Some(action) = action {
            resident.remember(self.tick, action);
        }
    }

    fn socialize(&mut self) {
        for index in 0..self.residents.len() {
            let (earlier, remaining) = self.residents.split_at_mut(index);
            let first = &mut remaining[0];
            for second in earlier {
                if first.active && second.active && first.position.distance(second.position) <= 1 {
                    let first_id = first.id;
                    for (resident, other) in [(&mut *first, second.id), (&mut *second, first_id)] {
                        let meetings = resident.relationships.entry(other).or_default();
                        *meetings = meetings.saturating_add(1);
                        resident.remember(self.tick, Action::Met { other });
                    }
                }
            }
        }
    }

    fn next_revision(&self, revision: u64) -> Result<u64, WorldError> {
        if revision != self.revision {
            return Err(WorldError::StaleRevision);
        }
        revision.checked_add(1).ok_or(WorldError::ClockExhausted)
    }

    fn contains(&self, position: Position) -> bool {
        position.x < self.width && position.z < self.depth
    }

    fn validate(&self) -> Result<(), WorldError> {
        if !(2..=64).contains(&self.width)
            || !(2..=64).contains(&self.depth)
            || !self.contains(self.depot)
            || self.tick > self.revision
            || self.shared_stock > MAX_SHARED
            || self.shelters > MAX_SHELTERS
            || !(1..=MAX_RESIDENTS).contains(&self.residents.len())
            || self.sites.len() > MAX_SITES
        {
            return Err(WorldError::Invalid("world bounds"));
        }
        let ids: BTreeSet<_> = self.residents.iter().map(|resident| resident.id).collect();
        if ids.len() != self.residents.len()
            || self.residents.iter().any(|resident| {
                resident.name.trim().is_empty()
                    || resident.name.len() > 64
                    || !self.contains(resident.position)
                    || resident.carried > MAX_CARRIED
                    || resident.memory.len() > MAX_MEMORY
                    || resident.memory.iter().any(|memory| memory.tick > self.tick)
                    || resident
                        .relationships
                        .keys()
                        .any(|id| *id == resident.id || !ids.contains(id))
            })
        {
            return Err(WorldError::Invalid("resident state"));
        }
        let sites: BTreeSet<_> = self.sites.iter().map(|site| site.position).collect();
        if sites.len() != self.sites.len()
            || self.sites.iter().any(|site| {
                !self.contains(site.position)
                    || site.capacity == 0
                    || site.capacity > MAX_STOCK
                    || site.stock > site.capacity
                    || site.regeneration > site.capacity
            })
        {
            return Err(WorldError::Invalid("resource site state"));
        }
        Ok(())
    }
}

fn walk(resident: &mut Resident, target: Position) -> Action {
    resident.position = resident.position.toward(target);
    Action::Moved {
        to: resident.position,
    }
}

fn deliver(resident: &mut Resident, depot: Position, shared_stock: &mut u16) -> Option<Action> {
    resident.goal = Goal::Deliver;
    if resident.position != depot {
        return Some(walk(resident, depot));
    }
    let amount = resident.carried.min(MAX_SHARED - *shared_stock);
    *shared_stock += amount;
    resident.carried -= amount;
    (amount > 0).then_some(Action::Delivered { amount })
}
