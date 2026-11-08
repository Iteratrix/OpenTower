//! Completeness oracle: an independent model of who can get where. The router builds a few
//! fixed route shapes by scanning columns; this models the tower as walkable floor runs joined
//! by shaft rides and stair or escalator flights, and answers three questions per tenant:
//!
//! - Is it physically connected to a ground lobby at all (any number of legs, any transfer)?
//! - Does one of the router's supported shapes reach it (restated here over floor runs)?
//! - If only the physical answer is yes, which missing shape would it need (a design gap)?
//!
//! It reads raw facilities, floor cells and shaft served floors; the only shared helpers are
//! geometry (`floor_ordinal`, `vertical_walkway_story_span`).

use std::collections::{HashMap, HashSet, VecDeque};

use crate::traffic::{
    ElevatorShaft, MAX_ESCALATOR_FLIGHTS, MAX_STAIR_FLIGHTS, TrafficSimulation, floor_from_ordinal,
    floor_ordinal, vertical_walkway_story_span,
};
use crate::{Facility, FacilityKind, Tower};

/// A run of contiguous floor cells: (floor, first column). People walk freely inside one.
type Run = (i16, u16);

#[derive(Clone, Copy, Debug)]
enum Leg {
    Ride {
        kind: FacilityKind,
        x: u16,
        from: i16,
        to: i16,
    },
    Climb {
        kind: FacilityKind,
        x: f32,
        from: i16,
        to: i16,
    },
}

impl Leg {
    fn ends(self) -> (i16, i16) {
        match self {
            Self::Ride { from, to, .. } | Self::Climb { from, to, .. } => (from, to),
        }
    }
}

impl std::fmt::Display for Leg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ride { kind, x, from, to } => write!(f, "{kind:?} x{x} {from}->{to}"),
            Self::Climb { kind, x, from, to } => write!(f, "{kind:?} x{x} {from}->{to}"),
        }
    }
}

/// What the oracle concludes about one tenant the router left unreachable.
pub(super) enum Verdict {
    /// A supported route shape exists; the router should have found it.
    RouterMissed(String),
    /// Physically connected, but only through a shape the router does not build.
    DesignGap {
        category: &'static str,
        path: String,
    },
    /// Not connected to a ground lobby at all.
    CutOff,
}

pub(super) struct Model<'a> {
    tower: &'a Tower,
    runs: HashMap<i16, Vec<(u16, u16)>>,
    lobbies: HashMap<i16, Vec<(u16, u16)>>,
    /// Flight base ordinals covered by each stair or escalator column.
    walkways: HashMap<(FacilityKind, u16), HashSet<i16>>,
    /// Public shafts and the floors their cars stop at.
    shafts: Vec<(&'a ElevatorShaft, HashSet<i16>)>,
    /// Multi-story lobbies as (floor, stories, first column, end column).
    grand_lobbies: Vec<(i16, u8, u16, u16)>,
    /// Centres of the ground-floor lobby slices people start from.
    ground_slices: Vec<f32>,
}

/// Sorted half-open runs of consecutive columns.
fn runs_of(mut columns: Vec<u16>) -> Vec<(u16, u16)> {
    columns.sort_unstable();
    columns.dedup();
    let mut runs: Vec<(u16, u16)> = Vec::new();
    for column in columns {
        match runs.last_mut() {
            Some(run) if run.1 == column => run.1 += 1,
            _ => runs.push((column, column + 1)),
        }
    }
    runs
}

fn is_walkway(kind: FacilityKind) -> bool {
    matches!(kind, FacilityKind::Stairs | FacilityKind::Escalator)
}

fn climb_limit(kind: FacilityKind) -> u16 {
    if kind == FacilityKind::Escalator {
        MAX_ESCALATOR_FLIGHTS
    } else {
        MAX_STAIR_FLIGHTS
    }
}

fn flights(from: i16, to: i16) -> u16 {
    (floor_ordinal(to) - floor_ordinal(from)).unsigned_abs()
}

impl<'a> Model<'a> {
    pub fn new(tower: &'a Tower, traffic: &'a TrafficSimulation) -> Self {
        let mut cells: HashMap<i16, Vec<u16>> = HashMap::new();
        for cell in tower.floors() {
            cells.entry(cell.floor).or_default().push(cell.x);
        }
        let mut lobby_cells: HashMap<i16, Vec<u16>> = HashMap::new();
        let mut walkways: HashMap<(FacilityKind, u16), HashSet<i16>> = HashMap::new();
        for facility in tower.facilities() {
            if facility.kind == FacilityKind::Lobby {
                lobby_cells
                    .entry(facility.position.floor)
                    .or_default()
                    .push(facility.position.x);
            } else if is_walkway(facility.kind) {
                let base = floor_ordinal(facility.position.floor);
                let span = vertical_walkway_story_span(facility, tower.facilities());
                let bases = walkways
                    .entry((facility.kind, facility.position.x))
                    .or_default();
                for flight in 0..span.saturating_sub(1) {
                    bases.insert(base + flight as i16);
                }
            }
        }
        let mut model = Self {
            tower,
            runs: cells
                .into_iter()
                .map(|(floor, xs)| (floor, runs_of(xs)))
                .collect(),
            lobbies: lobby_cells
                .into_iter()
                .map(|(floor, xs)| (floor, runs_of(xs)))
                .collect(),
            walkways,
            shafts: Vec::new(),
            grand_lobbies: tower
                .facilities()
                .iter()
                .filter(|lobby| lobby.kind == FacilityKind::Lobby && lobby.stories > 1)
                .map(|lobby| {
                    (
                        lobby.position.floor,
                        lobby.stories,
                        lobby.position.x,
                        lobby.end_x(),
                    )
                })
                .collect(),
            ground_slices: tower
                .facilities()
                .iter()
                .filter(|facility| {
                    facility.kind == FacilityKind::Lobby && facility.position.floor == 1
                })
                .map(|facility| f32::from(facility.position.x) + 0.5)
                .collect(),
        };
        model.shafts = traffic
            .elevators
            .iter()
            .filter(|shaft| shaft.kind != FacilityKind::ServiceElevator)
            .map(|shaft| {
                let stops = shaft
                    .served_floors
                    .iter()
                    .copied()
                    .filter(|floor| model.stops_at(shaft, *floor))
                    .collect();
                (shaft, stops)
            })
            .collect();
        model
    }

    fn run(&self, floor: i16, x: f32) -> Option<Run> {
        if x < 0.0 {
            return None;
        }
        let column = x.floor() as u16;
        self.runs
            .get(&floor)?
            .iter()
            .find(|(start, end)| *start <= column && column < *end)
            .map(|(start, _)| (floor, *start))
    }

    /// A middle story of a multi-story lobby, where nothing stops.
    fn inside_grand_lobby(&self, floor: i16, x: u16, width: u16) -> bool {
        self.grand_lobbies
            .iter()
            .any(|&(bottom, stories, start, end)| {
                floor > bottom
                    && floor < bottom + i16::from(stories) - 1
                    && start <= x + width
                    && end >= x
            })
    }

    /// A lobby run on `floor` touching the horizontal span `[start, end]`.
    fn lobby_touching(&self, floor: i16, start: f32, end: f32) -> impl Iterator<Item = (u16, u16)> {
        self.lobbies
            .get(&floor)
            .into_iter()
            .flatten()
            .copied()
            .filter(move |(left, right)| f32::from(*left) <= end && f32::from(*right) >= start)
    }

    fn shaft_span(shaft: &ElevatorShaft) -> (f32, f32) {
        let start = f32::from(shaft.x);
        (start, start + f32::from(shaft.kind.spec().width))
    }

    fn shaft_center(shaft: &ElevatorShaft) -> f32 {
        let (start, end) = Self::shaft_span(shaft);
        (start + end) * 0.5
    }

    fn public_shafts(&self) -> impl Iterator<Item = &'a ElevatorShaft> + '_ {
        self.shafts.iter().map(|(shaft, _)| *shaft)
    }

    fn stops(&self, shaft: &ElevatorShaft, floor: i16) -> bool {
        self.shafts
            .iter()
            .find(|(candidate, _)| candidate.id == shaft.id)
            .is_some_and(|(_, stops)| stops.contains(&floor))
    }

    /// Cars stop at a served floor outside a grand lobby's middle stories; express cars above
    /// ground stop only at the ground floor or a fifteenth-floor sky lobby touching the shaft.
    fn stops_at(&self, shaft: &ElevatorShaft, floor: i16) -> bool {
        let width = shaft.kind.spec().width;
        if !shaft.served_floors.contains(&floor) || self.inside_grand_lobby(floor, shaft.x, width) {
            return false;
        }
        if shaft.kind != FacilityKind::ExpressElevator || floor < 1 {
            return true;
        }
        let (start, end) = Self::shaft_span(shaft);
        (floor == 1 || floor % 15 == 0) && self.lobby_touching(floor, start, end).next().is_some()
    }

    fn walkway_center(kind: FacilityKind, x: u16) -> f32 {
        f32::from(x) + f32::from(kind.spec().width) * 0.5
    }

    /// Whether the column climbs without a break between two floors.
    fn column_connects(&self, kind: FacilityKind, x: u16, from: i16, to: i16) -> bool {
        let Some(bases) = self.walkways.get(&(kind, x)) else {
            return false;
        };
        let (low, high) = (
            floor_ordinal(from).min(floor_ordinal(to)),
            floor_ordinal(from).max(floor_ordinal(to)),
        );
        (low..high).all(|base| bases.contains(&base))
    }

    /// One walkway leg the router accepts: a single column within its climb limit, not ending
    /// in the middle of a grand lobby.
    fn walkway_leg(&self, kind: FacilityKind, x: u16, from: i16, to: i16) -> bool {
        let width = kind.spec().width;
        from != to
            && flights(from, to) <= climb_limit(kind)
            && self.column_connects(kind, x, from, to)
            && !self.inside_grand_lobby(from, x, width)
            && !self.inside_grand_lobby(to, x, width)
    }

    fn lobby_floors(&self) -> Vec<i16> {
        let mut floors = self.lobbies.keys().copied().collect::<Vec<_>>();
        floors.sort_unstable();
        floors
    }

    /// Whether someone starting on a ground lobby slice can walk to `x` on floor 1, and, when
    /// boarding `shaft`, from a slice whose lobby run touches it.
    fn reachable_from_ground(&self, x: f32, shaft: Option<&ElevatorShaft>) -> bool {
        let target = self.run(1, x);
        target.is_some()
            && self.ground_slices.iter().copied().any(|slice| {
                self.run(1, slice) == target
                    && shaft.is_none_or(|shaft| {
                        let (start, end) = Self::shaft_span(shaft);
                        self.lobby_touching(1, start, end).any(|(left, right)| {
                            f32::from(left) <= slice && slice < f32::from(right)
                        })
                    })
            })
    }

    /// The router's supported shapes, restated over floor runs. Returns the first shape found.
    fn supported_route(&self, tenant: &Facility) -> Option<String> {
        let floor = tenant.position.floor;
        let x = f32::from(tenant.position.x) + f32::from(tenant.kind.spec().width) * 0.5;
        let goal = self.run(floor, x)?;
        let arrives = |via: f32| self.run(floor, via) == Some(goal);
        if floor == 1 {
            return self
                .reachable_from_ground(x, None)
                .then(|| "walk".to_owned());
        }
        let columns = self.walkways.keys().copied().collect::<Vec<_>>();
        for &(kind, column) in &columns {
            let center = Self::walkway_center(kind, column);
            if self.walkway_leg(kind, column, 1, floor)
                && self.reachable_from_ground(center, None)
                && arrives(center)
            {
                return Some(format!("{kind:?} x{column}"));
            }
        }
        let boards = |shaft: &ElevatorShaft| {
            self.stops(shaft, 1)
                && self.reachable_from_ground(Self::shaft_center(shaft), Some(shaft))
        };
        let alights =
            |shaft: &ElevatorShaft| self.stops(shaft, floor) && arrives(Self::shaft_center(shaft));
        for shaft in self.public_shafts() {
            if boards(shaft) && alights(shaft) {
                return Some(format!("{:?} x{}", shaft.kind, shaft.x));
            }
        }
        let lobby_floors = self.lobby_floors();
        let joins = |level: i16, (a_start, a_end): (f32, f32), (b_start, b_end): (f32, f32)| {
            self.lobby_touching(level, a_start, a_end)
                .any(|(left, right)| f32::from(left) <= b_end && f32::from(right) >= b_start)
        };
        for first in self.public_shafts().filter(|shaft| boards(shaft)) {
            for second in self
                .public_shafts()
                .filter(|shaft| shaft.id != first.id && alights(shaft))
            {
                for &level in &lobby_floors {
                    if self.stops(first, level)
                        && self.stops(second, level)
                        && joins(level, Self::shaft_span(first), Self::shaft_span(second))
                    {
                        return Some(format!(
                            "{:?} x{} to {level}, {:?} x{}",
                            first.kind, first.x, second.kind, second.x
                        ));
                    }
                }
            }
        }
        for &level in lobby_floors
            .iter()
            .filter(|level| **level != 1 && **level != floor)
        {
            for &(kind, column) in &columns {
                let center = Self::walkway_center(kind, column);
                let half = f32::from(kind.spec().width) * 0.5;
                let walkway_span = (center - half, center + half);
                let changes = |shaft: &ElevatorShaft| {
                    self.stops(shaft, level)
                        && joins(level, Self::shaft_span(shaft), walkway_span)
                        && self.run(level, Self::shaft_center(shaft)).is_some()
                        && self.run(level, Self::shaft_center(shaft)) == self.run(level, center)
                };
                if self.walkway_leg(kind, column, level, floor)
                    && arrives(center)
                    && let Some(shaft) = self
                        .public_shafts()
                        .find(|shaft| boards(shaft) && changes(shaft))
                {
                    return Some(format!(
                        "{:?} x{} to {level}, {kind:?} x{column}",
                        shaft.kind, shaft.x
                    ));
                }
                if self.walkway_leg(kind, column, 1, level)
                    && self.reachable_from_ground(center, None)
                    && let Some(shaft) = self
                        .public_shafts()
                        .find(|shaft| alights(shaft) && changes(shaft))
                {
                    return Some(format!(
                        "{kind:?} x{column} to {level}, {:?} x{}",
                        shaft.kind, shaft.x
                    ));
                }
            }
        }
        None
    }

    /// Fewest-leg physical paths from the ground lobby runs: any shaft between any two of its
    /// stops, any unbroken climb of one stair or escalator column, transfers anywhere.
    fn physical_paths(&self) -> HashMap<Run, Option<(Run, Leg)>> {
        let mut parent: HashMap<Run, Option<(Run, Leg)>> = HashMap::new();
        let mut queue = VecDeque::new();
        for &slice in &self.ground_slices {
            if let Some(run) = self.run(1, slice)
                && parent.insert(run, None).is_none()
            {
                queue.push_back(run);
            }
        }
        while let Some(here) = queue.pop_front() {
            let (floor, start) = here;
            let end = self.runs[&floor]
                .iter()
                .find(|(left, _)| *left == start)
                .map_or(start, |(_, right)| *right);
            let on_run = |x: f32| x >= f32::from(start) && x.floor() < f32::from(end);
            let mut next = Vec::new();
            for (shaft, stops) in &self.shafts {
                let center = Self::shaft_center(shaft);
                if !on_run(center) || !stops.contains(&floor) {
                    continue;
                }
                for &to in stops {
                    if to != floor
                        && let Some(run) = self.run(to, center)
                    {
                        next.push((
                            run,
                            Leg::Ride {
                                kind: shaft.kind,
                                x: shaft.x,
                                from: floor,
                                to,
                            },
                        ));
                    }
                }
            }
            for (&(kind, column), bases) in &self.walkways {
                let center = Self::walkway_center(kind, column);
                if !on_run(center) {
                    continue;
                }
                for step in [1i16, -1] {
                    let mut ordinal = floor_ordinal(floor);
                    loop {
                        let base = if step > 0 { ordinal } else { ordinal - 1 };
                        if !bases.contains(&base) {
                            break;
                        }
                        ordinal += step;
                        let to = floor_from_ordinal(ordinal);
                        if self.inside_grand_lobby(to, column, kind.spec().width) {
                            continue;
                        }
                        if let Some(run) = self.run(to, center) {
                            next.push((
                                run,
                                Leg::Climb {
                                    kind,
                                    x: center,
                                    from: floor,
                                    to,
                                },
                            ));
                        }
                    }
                }
            }
            for (run, leg) in next {
                if let std::collections::hash_map::Entry::Vacant(entry) = parent.entry(run) {
                    entry.insert(Some((here, leg)));
                    queue.push_back(run);
                }
            }
        }
        parent
    }

    /// Tenants the router leaves unreachable, each with the oracle's verdict, and routed tenants
    /// that are not physically connected at all.
    pub fn audit(
        &self,
        traffic: &TrafficSimulation,
    ) -> (Vec<(&'a Facility, Verdict)>, Vec<&'a Facility>) {
        let paths = self.physical_paths();
        let mut unrouted = Vec::new();
        let mut cut_off_but_routed = Vec::new();
        for tenant in self
            .tower
            .facilities()
            .iter()
            .filter(|facility| facility.kind.has_tenant_occupancy())
        {
            let x = f32::from(tenant.position.x) + f32::from(tenant.kind.spec().width) * 0.5;
            let run = self.run(tenant.position.floor, x);
            let connected = run.is_some_and(|run| paths.contains_key(&run));
            if traffic.reachable_facility_set().contains(&tenant.id) {
                if !connected {
                    cut_off_but_routed.push(tenant);
                }
                continue;
            }
            let verdict = if let Some(shape) = self.supported_route(tenant) {
                Verdict::RouterMissed(shape)
            } else if let Some(run) = run.filter(|_| connected) {
                let mut legs = Vec::new();
                let mut at = run;
                while let Some(Some((previous, leg))) = paths.get(&at) {
                    legs.push(*leg);
                    at = *previous;
                }
                legs.reverse();
                Verdict::DesignGap {
                    category: self.categorize(&legs),
                    path: legs
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", "),
                }
            } else {
                Verdict::CutOff
            };
            unrouted.push((tenant, verdict));
        }
        (unrouted, cut_off_but_routed)
    }

    /// Why a fewest-leg physical path falls outside the supported shapes (first reason found;
    /// another path with as many legs may fail for a different reason).
    fn categorize(&self, legs: &[Leg]) -> &'static str {
        let lobby_floors = self.lobby_floors();
        if legs.len() >= 3 {
            return "needs three or more legs";
        }
        if legs.iter().any(|leg| match *leg {
            Leg::Climb { kind, from, to, .. } => flights(from, to) > climb_limit(kind),
            Leg::Ride { .. } => false,
        }) {
            return "needs a longer stair or escalator climb than people accept";
        }
        if let [first, second] = legs {
            let change = first.ends().1;
            if matches!((first, second), (Leg::Climb { .. }, Leg::Climb { .. })) {
                return "needs two stair or escalator columns";
            }
            if !lobby_floors.contains(&change) {
                return "changes transport on a floor without a lobby";
            }
            return "changes transport where no one lobby joins both";
        }
        if matches!(legs.first(), Some(Leg::Ride { .. })) {
            return "boards a shaft no ground lobby touches";
        }
        "unclassified"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GridPosition;

    fn tower_with_floors(top: i16, width: u16) -> Tower {
        let mut tower = Tower::new(crate::DEFAULT_TOWER_WIDTH, 10_000_000_000);
        for floor in 1..=top {
            for x in 0..width {
                tower.place_floor(GridPosition { x, floor }).unwrap();
            }
        }
        for x in 0..width {
            tower
                .place(FacilityKind::Lobby, GridPosition { x, floor: 1 })
                .unwrap();
        }
        tower
    }

    fn audit(tower: &Tower) -> Vec<(u64, String)> {
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(tower);
        let model = Model::new(tower, &traffic);
        let (unrouted, cut_off) = model.audit(&traffic);
        assert!(cut_off.is_empty());
        unrouted
            .into_iter()
            .map(|(tenant, verdict)| {
                let label = match verdict {
                    Verdict::RouterMissed(shape) => format!("missed {shape}"),
                    Verdict::DesignGap { category, .. } => category.to_owned(),
                    Verdict::CutOff => "cut off".to_owned(),
                };
                (tenant.id, label)
            })
            .collect()
    }

    #[test]
    fn offset_stairs_are_a_design_gap_and_a_floating_office_is_cut_off() {
        let mut tower = tower_with_floors(4, 40);
        for (x, floor) in [(0, 1), (0, 2), (20, 3)] {
            tower
                .place(FacilityKind::Stairs, GridPosition { x, floor })
                .unwrap();
        }
        let up_offset_stairs = tower
            .place(FacilityKind::Office, GridPosition { x: 30, floor: 4 })
            .unwrap();
        for x in 0..10 {
            tower
                .place_floor(GridPosition {
                    x: 50 + x,
                    floor: 1,
                })
                .unwrap();
            tower
                .place_floor(GridPosition {
                    x: 50 + x,
                    floor: 2,
                })
                .unwrap();
        }
        let floating = tower
            .place(FacilityKind::Office, GridPosition { x: 50, floor: 2 })
            .unwrap();
        let mut verdicts = audit(&tower);
        verdicts.sort_unstable();
        assert_eq!(
            verdicts,
            vec![
                (
                    up_offset_stairs,
                    "needs two stair or escalator columns".to_owned()
                ),
                (floating, "cut off".to_owned()),
            ]
        );
    }

    #[test]
    fn routed_towers_have_no_verdicts() {
        let mut tower = tower_with_floors(6, 40);
        for floor in 1..=6 {
            tower
                .place(FacilityKind::Elevator, GridPosition { x: 4, floor })
                .unwrap();
        }
        for floor in [3, 6] {
            tower
                .place(FacilityKind::Office, GridPosition { x: 20, floor })
                .unwrap();
        }
        assert_eq!(audit(&tower), Vec::new());
    }
}
