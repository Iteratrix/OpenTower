//! Operation-sequence property tests over small generated towers. Each case starts from a
//! seeded tower (lobby, floors, one shaft, two offices) and applies a random history of edits,
//! speed and schedule changes, save/load round trips and frame runs, with the invariant
//! battery checked after every edit and every frame. Operation mixes are weighted per case
//! (swarm testing), and operands are chosen from the current state by index so shrinking keeps
//! them meaningful.
//!
//! Knobs: DISCOVERY_CASES (default 64), DISCOVERY_IGNORE (comma-separated violation kinds to
//! skip, default the known findings; "none" to check everything).

use proptest::prelude::*;
use proptest::strategy::Union;
use proptest::test_runner::TestCaseError;

use super::{FRAME_SECONDS, Monitor, frame, load_save};
use crate::traffic::{
    ElevatorMode, Endpoint, MAX_SIMULATED_PEOPLE, Person, PersonActivity, PersonMood, PersonRole,
    TrafficSimulation, TravelMode, endpoint, floor_ordinal, shaft_center, shaft_stops_at,
};
use crate::{FacilityKind, GridPosition, SimulationSpeed, Tower};

const WIDTH: u16 = 64;
const KINDS: [FacilityKind; 9] = [
    FacilityKind::Elevator,
    FacilityKind::ExpressElevator,
    FacilityKind::ServiceElevator,
    FacilityKind::Stairs,
    FacilityKind::Escalator,
    FacilityKind::Office,
    FacilityKind::Condo,
    FacilityKind::FastFood,
    FacilityKind::HotelSingle,
];
const SPEEDS: [SimulationSpeed; 5] = [
    SimulationSpeed::Normal,
    SimulationSpeed::Fast,
    SimulationSpeed::Triple,
    SimulationSpeed::Quintuple,
    SimulationSpeed::Tenfold,
];
const MODES: [ElevatorMode; 3] = [
    ElevatorMode::Local,
    ElevatorMode::ExpressToTop,
    ElevatorMode::ExpressToBottom,
];
const KNOWN: [&str; 2] = ["idle car beside a waiter", "shaft has a gap"];

/// Edits aim at existing structure (an anchor picked by index from the current state) so most
/// of them are legal and land where traffic already flows; a few absolute placements keep
/// some diversity.
#[derive(Clone, Debug)]
enum Op {
    FloorRun {
        anchor: usize,
        up: bool,
        len: u16,
    },
    Place {
        kind: usize,
        anchor: usize,
        dx: i16,
        dfloor: i16,
    },
    PlaceAt {
        kind: usize,
        x: u16,
        floor: i16,
    },
    Lobby {
        x: u16,
        floor: i16,
        stories: u8,
    },
    Demolish {
        pick: usize,
    },
    DemolishFloor {
        pick: usize,
    },
    AddCar {
        pick: usize,
        floor: i16,
    },
    Schedule {
        pick: usize,
        mode: usize,
    },
    Speed {
        speed: usize,
    },
    Advance {
        frames: u16,
    },
    SaveLoad,
    /// Up to `count` people waiting at one served floor of one shaft, bound for tenants on
    /// another of its floors, to reach car-capacity and queue-limit edges that a small tower's
    /// natural traffic rarely does.
    Crowd {
        shaft: usize,
        floor: usize,
        tenant: usize,
        count: u8,
    },
    /// One more segment at the top or bottom of an existing shaft.
    ExtendShaft {
        shaft: usize,
        up: bool,
    },
}

fn op_strategy(weights: [u32; 13]) -> BoxedStrategy<Op> {
    let options: Vec<(u32, BoxedStrategy<Op>)> = vec![
        (
            weights[0],
            (
                any::<usize>(),
                prop_oneof![3 => Just(true), 1 => Just(false)],
                1u16..=24,
            )
                .prop_map(|(anchor, up, len)| Op::FloorRun { anchor, up, len })
                .boxed(),
        ),
        (
            weights[1],
            (
                0..KINDS.len(),
                any::<usize>(),
                -12i16..=12,
                prop_oneof![2 => Just(0i16), 3 => -2i16..=2],
            )
                .prop_map(|(kind, anchor, dx, dfloor)| Op::Place {
                    kind,
                    anchor,
                    dx,
                    dfloor,
                })
                .boxed(),
        ),
        (
            weights[2],
            (0..KINDS.len(), 0u16..WIDTH, -3i16..=16)
                .prop_map(|(kind, x, floor)| Op::PlaceAt { kind, x, floor })
                .boxed(),
        ),
        (
            weights[3],
            (0u16..WIDTH, prop_oneof![Just(1i16), Just(15i16)], 1u8..=3)
                .prop_map(|(x, floor, stories)| Op::Lobby { x, floor, stories })
                .boxed(),
        ),
        (
            weights[4],
            any::<usize>()
                .prop_map(|pick| Op::Demolish { pick })
                .boxed(),
        ),
        (
            weights[5],
            any::<usize>()
                .prop_map(|pick| Op::DemolishFloor { pick })
                .boxed(),
        ),
        (
            weights[6],
            (any::<usize>(), -3i16..=16)
                .prop_map(|(pick, floor)| Op::AddCar { pick, floor })
                .boxed(),
        ),
        (
            weights[7],
            (any::<usize>(), 0..MODES.len())
                .prop_map(|(pick, mode)| Op::Schedule { pick, mode })
                .boxed(),
        ),
        (
            weights[8],
            prop_oneof![1 => 0..SPEEDS.len(), 2 => Just(SPEEDS.len() - 1)]
                .prop_map(|speed| Op::Speed { speed })
                .boxed(),
        ),
        (
            weights[9],
            prop_oneof![3 => 1u16..=600, 1 => 600u16..=7200]
                .prop_map(|frames| Op::Advance { frames })
                .boxed(),
        ),
        (weights[10], Just(Op::SaveLoad).boxed()),
        (
            weights[11],
            (any::<usize>(), any::<usize>(), any::<usize>(), 1u8..=40)
                .prop_map(|(shaft, floor, tenant, count)| Op::Crowd {
                    shaft,
                    floor,
                    tenant,
                    count,
                })
                .boxed(),
        ),
        (
            weights[12],
            (any::<usize>(), any::<bool>())
                .prop_map(|(shaft, up)| Op::ExtendShaft { shaft, up })
                .boxed(),
        ),
    ];
    Union::new_weighted(
        options
            .into_iter()
            .filter(|(weight, _)| *weight > 0)
            .collect(),
    )
    .boxed()
}

/// A history: per-case operation weights (anchored placement and Advance always enabled), then
/// the ops.
fn history() -> impl Strategy<Value = Vec<Op>> {
    proptest::array::uniform13(0u32..=6).prop_flat_map(|mut weights| {
        weights[1] = weights[1].max(2);
        weights[9] = weights[9].max(2);
        proptest::collection::vec(op_strategy(weights), 1..48)
    })
}

/// Floors 1-10 across the site and 11-16 over its left part, a full ground lobby and a sky lobby
/// on 15, a standard, a service and an express shaft, stairs up to floor 4 and from the sky
/// lobby to 12 and 16, and tenants reachable directly, by stairs, and only through the sky
/// lobby. Run three game hours at Tenfold so tenants move in.
fn seeded_tower() -> (Tower, TrafficSimulation) {
    // The site is the standard width (loading widens narrower sites); building stays in 0..WIDTH.
    let mut tower = Tower::new(crate::DEFAULT_TOWER_WIDTH, 10_000_000_000);
    for floor in 1..=10 {
        for x in 0..WIDTH {
            tower.place_floor(GridPosition { x, floor }).unwrap();
        }
    }
    for floor in 11..=16 {
        for x in 0..40 {
            tower.place_floor(GridPosition { x, floor }).unwrap();
        }
    }
    for x in 0..WIDTH {
        tower
            .place(FacilityKind::Lobby, GridPosition { x, floor: 1 })
            .unwrap();
    }
    for x in 0..32 {
        tower
            .place(FacilityKind::Lobby, GridPosition { x, floor: 15 })
            .unwrap();
    }
    for floor in 1..=15 {
        tower
            .place(FacilityKind::ExpressElevator, GridPosition { x: 32, floor })
            .unwrap();
    }
    for floor in [12, 13, 14, 15] {
        tower
            .place(FacilityKind::Stairs, GridPosition { x: 20, floor })
            .unwrap();
    }
    for floor in 1..=10 {
        tower
            .place(FacilityKind::Elevator, GridPosition { x: 8, floor })
            .unwrap();
        tower
            .place(FacilityKind::ServiceElevator, GridPosition { x: 56, floor })
            .unwrap();
    }
    for floor in 1..=3 {
        tower
            .place(FacilityKind::Stairs, GridPosition { x: 16, floor })
            .unwrap();
    }
    for (kind, x, floor) in [
        (FacilityKind::Office, 24, 2),
        (FacilityKind::Office, 24, 4),
        (FacilityKind::Office, 24, 7),
        (FacilityKind::Condo, 34, 3),
        (FacilityKind::Condo, 34, 8),
        (FacilityKind::FastFood, 34, 5),
        (FacilityKind::HotelSingle, 24, 6),
        (FacilityKind::HotelSingle, 28, 6),
        (FacilityKind::HotelSingle, 24, 9),
        (FacilityKind::Condo, 2, 12),
        (FacilityKind::Office, 2, 16),
        (FacilityKind::HotelSingle, 2, 14),
    ] {
        tower.place(kind, GridPosition { x, floor }).unwrap();
    }
    let mut traffic = TrafficSimulation::new();
    traffic.sync_with_tower(&tower);
    tower.clock.speed = SimulationSpeed::Tenfold;
    for _ in 0..540 {
        frame(&mut tower, &mut traffic, FRAME_SECONDS);
    }
    (tower, traffic)
}

fn skip_floor_zero(floor: i16) -> i16 {
    if floor == 0 { 1 } else { floor }
}

fn ignored_kinds() -> Vec<String> {
    match std::env::var("DISCOVERY_IGNORE") {
        Ok(list) if list == "none" => Vec::new(),
        Ok(list) => list.split(',').map(str::to_owned).collect(),
        Err(_) => KNOWN.iter().map(|kind| (*kind).to_owned()).collect(),
    }
}

struct Run {
    tower: Tower,
    traffic: TrafficSimulation,
    monitor: Monitor,
    ignored: Vec<String>,
    stats: Stats,
}

/// What a history actually exercised, for checking generator coverage.
#[derive(Clone, Debug, Default)]
struct Stats {
    applied: std::collections::BTreeMap<&'static str, (u32, u32)>,
    frames: u64,
    max_people: usize,
    max_riders: usize,
    max_waiting: usize,
    rider_frames: u64,
    occupied_tenants: usize,
    shafts: usize,
    max_cars: usize,
    game_minutes: u64,
}

impl Run {
    fn new() -> Self {
        let (tower, traffic) = seeded_tower();
        Self {
            tower,
            traffic,
            monitor: Monitor::default(),
            ignored: ignored_kinds(),
            stats: Stats::default(),
        }
    }

    fn check(&mut self, dt: f32, step: usize, op: &Op) -> Result<(), TestCaseError> {
        let violations = self.monitor.check(&self.tower, &self.traffic, dt);
        match violations.into_iter().find(|violation| {
            !self
                .ignored
                .iter()
                .any(|kind| kind == "*" || kind == violation.kind)
        }) {
            None => Ok(()),
            Some(violation) => Err(TestCaseError::fail(format!(
                "{} after op {step} {op:?} (t={:.1}s, day {} {:02}:{:02}): {}",
                violation.kind,
                self.monitor.elapsed,
                self.tower.clock.day,
                self.tower.clock.minute_of_day / 60,
                self.tower.clock.minute_of_day % 60,
                violation.detail
            ))),
        }
    }

    fn apply(&mut self, step: usize, op: &Op) -> Result<(), TestCaseError> {
        let before = (
            self.tower.facilities().len(),
            self.tower.floors().len(),
            self.traffic
                .elevators()
                .iter()
                .map(|shaft| shaft.cars.len())
                .sum::<usize>(),
        );
        let result = self.apply_inner(step, op);
        if result.is_err() {
            return result;
        }
        let after = (
            self.tower.facilities().len(),
            self.tower.floors().len(),
            self.traffic
                .elevators()
                .iter()
                .map(|shaft| shaft.cars.len())
                .sum::<usize>(),
        );
        let name = match op {
            Op::FloorRun { .. } => "floor run",
            Op::Place { kind, .. } | Op::PlaceAt { kind, .. } => match KINDS[*kind] {
                FacilityKind::Elevator
                | FacilityKind::ExpressElevator
                | FacilityKind::ServiceElevator => "place shaft",
                FacilityKind::Stairs | FacilityKind::Escalator => "place walkway",
                _ => "place tenant",
            },
            Op::Lobby { .. } => "lobby",
            Op::Demolish { .. } => "demolish",
            Op::DemolishFloor { .. } => "demolish floor",
            Op::AddCar { .. } => "add car",
            Op::Schedule { .. } => "schedule",
            Op::Speed { .. } => "speed",
            Op::Advance { .. } => "advance",
            Op::SaveLoad => "save/load",
            Op::Crowd { .. } => "crowd",
            Op::ExtendShaft { .. } => "extend shaft",
        };
        let entry = self.stats.applied.entry(name).or_default();
        entry.0 += 1;
        entry.1 += u32::from(
            before != after
                || matches!(
                    op,
                    Op::Advance { .. } | Op::Speed { .. } | Op::Schedule { .. } | Op::SaveLoad
                ),
        );
        self.stats.occupied_tenants = self
            .tower
            .facilities()
            .iter()
            .filter(|facility| facility.is_occupied())
            .count();
        self.stats.shafts = self.traffic.elevators().len();
        self.stats.max_cars = self.stats.max_cars.max(after.2);
        self.stats.game_minutes = self.tower.clock.absolute_minute() - 8 * 60;
        result?;
        if before.0 != after.0 || before.1 != after.1 {
            self.compare_with_rebuild(step, op)?;
        }
        Ok(())
    }

    /// Recomputation oracle: after an edit, the incrementally maintained reachable set must
    /// match a from-scratch rebuild of the same tower.
    fn compare_with_rebuild(&self, step: usize, op: &Op) -> Result<(), TestCaseError> {
        if self
            .ignored
            .iter()
            .any(|kind| kind == "*" || kind == "incremental reachability differs")
        {
            return Ok(());
        }
        let mut fresh = TrafficSimulation::new();
        fresh.sync_with_tower(&self.tower);
        let cached = self.traffic.reachable_facility_set();
        let rebuilt = fresh.reachable_facility_set();
        if cached == rebuilt {
            return Ok(());
        }
        let describe = |ids: Vec<&u64>, source: &TrafficSimulation| {
            ids.into_iter()
                .map(|id| {
                    let facility = self
                        .tower
                        .facilities()
                        .iter()
                        .find(|facility| facility.id == *id);
                    format!(
                        "{id} {:?} via {:?}",
                        facility.map(|f| (f.kind, f.position)),
                        source.routes_from_lobby.get(id)
                    )
                })
                .collect::<Vec<_>>()
        };
        Err(TestCaseError::fail(format!(
            "incremental reachability differs after op {step} {op:?}: only cached {:?}; only rebuilt {:?}",
            describe(cached.difference(rebuilt).collect(), &self.traffic),
            describe(rebuilt.difference(cached).collect(), &fresh),
        )))
    }

    fn apply_inner(&mut self, step: usize, op: &Op) -> Result<(), TestCaseError> {
        let (tower, traffic) = (&mut self.tower, &mut self.traffic);
        match *op {
            Op::FloorRun { anchor, up, len } => {
                let floors = tower.floors();
                if !floors.is_empty() {
                    let anchor = floors[anchor % floors.len()];
                    let floor = match (up, anchor.floor) {
                        (true, -1) => 1,
                        (true, floor) => floor + 1,
                        (false, 1) => -1,
                        (false, floor) => floor - 1,
                    };
                    let start = anchor.x.saturating_sub(len / 2);
                    let mut placed = false;
                    for x in start..(start + len).min(WIDTH) {
                        placed |= tower.place_floor(GridPosition { x, floor }).is_ok();
                    }
                    if placed {
                        traffic.sync_with_tower_after_addition(tower);
                    }
                }
            }
            Op::Place {
                kind,
                anchor,
                dx,
                dfloor,
            } => {
                let facilities = tower.facilities();
                if !facilities.is_empty() {
                    let anchor = facilities[anchor % facilities.len()].clone();
                    let kind = KINDS[kind];
                    let same_shaft = anchor.kind == kind && dx.abs() < 4;
                    let x = if same_shaft {
                        anchor.position.x
                    } else {
                        (i32::from(anchor.position.x) + i32::from(dx))
                            .clamp(0, i32::from(WIDTH) - 1) as u16
                    };
                    let floor = skip_floor_zero(anchor.position.floor + dfloor);
                    if tower.place(kind, GridPosition { x, floor }).is_ok() {
                        traffic.sync_with_tower_after_addition(tower);
                    }
                }
            }
            Op::PlaceAt { kind, x, floor } => {
                if tower
                    .place(
                        KINDS[kind],
                        GridPosition {
                            x,
                            floor: skip_floor_zero(floor),
                        },
                    )
                    .is_ok()
                {
                    traffic.sync_with_tower_after_addition(tower);
                }
            }
            Op::Lobby { x, floor, stories } => {
                if tower
                    .place_lobby(GridPosition { x, floor }, stories)
                    .is_ok()
                {
                    traffic.sync_with_tower_after_addition(tower);
                }
            }
            Op::Demolish { pick } => {
                let facilities = tower.facilities();
                if !facilities.is_empty() {
                    let id = facilities[pick % facilities.len()].id;
                    if let Ok(removed) = tower.demolish(id) {
                        traffic.sync_with_tower_after_removal(tower, &removed);
                    }
                }
            }
            Op::DemolishFloor { pick } => {
                let floors = tower.floors();
                if !floors.is_empty() {
                    let position = floors[pick % floors.len()];
                    if tower.demolish_floor(position).is_ok() {
                        traffic.sync_with_tower_after_floor_removal(tower, position);
                    }
                }
            }
            Op::AddCar { pick, floor } => {
                if !traffic.elevators().is_empty() {
                    let shaft_id = traffic.elevators()[pick % traffic.elevators().len()].id;
                    traffic.add_elevator_car(tower, shaft_id, floor);
                }
            }
            Op::Schedule { pick, mode } => {
                if !traffic.elevators().is_empty() {
                    let shaft_id = traffic.elevators()[pick % traffic.elevators().len()].id;
                    let clock = tower.clock;
                    if let Some(shaft) = traffic.elevator_mut(shaft_id) {
                        // DISCOVERY_LOCAL_SCHEDULES keeps express modes out (finding 2).
                        let mode = if std::env::var("DISCOVERY_LOCAL_SCHEDULES").is_ok() {
                            0
                        } else {
                            mode
                        };
                        shaft.current_schedule_mut(clock).mode = MODES[mode];
                    }
                }
            }
            Op::Speed { speed } => tower.clock.speed = SPEEDS[speed],
            Op::Advance { frames } => {
                for _ in 0..frames {
                    let dt = frame(&mut self.tower, &mut self.traffic, FRAME_SECONDS);
                    let people = self.traffic.people();
                    let riders = people
                        .iter()
                        .filter(|person| {
                            matches!(
                                person.activity,
                                crate::traffic::PersonActivity::RidingElevator { .. }
                            )
                        })
                        .count();
                    let waiting = people
                        .iter()
                        .filter(|person| {
                            matches!(
                                person.activity,
                                crate::traffic::PersonActivity::WaitingForElevator { .. }
                            )
                        })
                        .count();
                    self.stats.frames += 1;
                    self.stats.max_people = self.stats.max_people.max(people.len());
                    self.stats.max_riders = self.stats.max_riders.max(riders);
                    self.stats.max_waiting = self.stats.max_waiting.max(waiting);
                    self.stats.rider_frames += riders as u64;
                    self.check(dt, step, op)?;
                }
                return Ok(());
            }
            Op::Crowd {
                shaft,
                floor,
                tenant,
                count,
            } => {
                if !traffic.elevators.is_empty() {
                    let shaft = traffic.elevators[shaft % traffic.elevators.len()].clone();
                    let landings = shaft
                        .served_floors
                        .iter()
                        .copied()
                        .filter(|landing| shaft_stops_at(&shaft, *landing, tower.facilities()))
                        .collect::<Vec<_>>();
                    let tenants = tower
                        .facilities()
                        .iter()
                        .filter(|facility| {
                            facility.kind.has_tenant_occupancy()
                                && landings.contains(&facility.position.floor)
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    if !landings.is_empty() && !tenants.is_empty() {
                        let start = landings[floor % landings.len()];
                        let target = &tenants[tenant % tenants.len()];
                        let shaft_x = shaft_center(&shaft);
                        if target.position.floor != start {
                            for _ in 0..count {
                                if traffic.people.len() >= MAX_SIMULATED_PEOPLE {
                                    break;
                                }
                                let id = traffic.next_person_id;
                                traffic.next_person_id += 1;
                                traffic.people.push(Person {
                                    id,
                                    role: PersonRole::Visitor,
                                    home_facility_id: 0,
                                    destination_facility_id: target.id,
                                    x: shaft_x,
                                    floor_position: f32::from(floor_ordinal(start)),
                                    current_floor: start,
                                    mood: PersonMood::Calm,
                                    activity: PersonActivity::WaitingForElevator {
                                        shaft_id: shaft.id,
                                    },
                                    wait_seconds: 0.0,
                                    origin: Endpoint {
                                        facility_id: 0,
                                        x: shaft_x,
                                        floor: start,
                                    },
                                    destination: endpoint(target),
                                    route: TravelMode::Elevator {
                                        shaft_id: shaft.id,
                                        x: shaft_x,
                                    },
                                    going_home: false,
                                    walking_from_transport: false,
                                    visit_remaining: 0.0,
                                    visit_duration: 20.0,
                                    stair_progress: 0.0,
                                    route_leg: 0,
                                });
                            }
                        }
                    }
                }
            }
            Op::ExtendShaft { shaft, up } => {
                if !traffic.elevators.is_empty() {
                    let shaft = &traffic.elevators[shaft % traffic.elevators.len()];
                    let (Some(&bottom), Some(&top)) =
                        (shaft.served_floors.first(), shaft.served_floors.last())
                    else {
                        return Ok(());
                    };
                    let floor = if up {
                        skip_floor_zero(top + 1)
                    } else if bottom == 1 {
                        -1
                    } else {
                        bottom - 1
                    };
                    if tower
                        .place(shaft.kind, GridPosition { x: shaft.x, floor })
                        .is_ok()
                    {
                        traffic.sync_with_tower_after_addition(tower);
                    }
                }
            }
            Op::SaveLoad => {
                let json =
                    serde_json::json!({ "tower": &*tower, "traffic": &*traffic }).to_string();
                let (loaded_tower, loaded_traffic) = load_save(&json).map_err(|error| {
                    TestCaseError::fail(format!("reload after op {step}: {error}"))
                })?;
                *tower = loaded_tower;
                *traffic = loaded_traffic;
            }
        }
        self.check(0.0, step, op)
    }
}

fn cases() -> u32 {
    std::env::var("DISCOVERY_CASES").map_or(64, |cases| cases.parse().unwrap())
}

fn state_json(run: &Run) -> serde_json::Value {
    let mut state = serde_json::json!({ "tower": &run.tower, "traffic": &run.traffic });
    // HashSet fields serialise in hash order; sort them so equal sets compare equal.
    for field in [
        "arrived_facilities",
        "reachable_facilities",
        "connected_parking",
        "known_destinations",
    ] {
        if let Some(serde_json::Value::Array(items)) = state["traffic"].get_mut(field) {
            items.sort_by_key(|item| item.as_u64());
        }
    }
    state
}

/// The first path at which two states differ, with both values (truncated).
fn first_difference(
    left: &serde_json::Value,
    right: &serde_json::Value,
    path: &str,
) -> Option<String> {
    use serde_json::Value;
    match (left, right) {
        (Value::Object(a), Value::Object(b)) => {
            let keys: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
            keys.into_iter()
                .find_map(|key| match (a.get(key), b.get(key)) {
                    (Some(x), Some(y)) => first_difference(x, y, &format!("{path}.{key}")),
                    (x, y) => Some(format!("{path}.{key}: {x:?} vs {y:?}")),
                })
        }
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => a
            .iter()
            .zip(b)
            .enumerate()
            .find_map(|(index, (x, y))| first_difference(x, y, &format!("{path}[{index}]"))),
        _ if left == right => None,
        _ => {
            let show = |value: &Value| value.to_string().chars().take(300).collect::<String>();
            Some(format!("{path}: {} vs {}", show(left), show(right)))
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: cases(),
        failure_persistence: None,
        max_shrink_iters: std::env::var("DISCOVERY_SHRINK").map_or(4096, |iters| iters.parse().unwrap()),
        ..ProptestConfig::default()
    })]

    #[test]
    fn random_histories_keep_the_invariants(ops in history()) {
        let mut run = Run::new();
        for (step, op) in ops.iter().enumerate() {
            run.apply(step, op)?;
        }
    }

    #[test]
    fn the_same_history_gives_the_same_state(ops in history()) {
        let mut first = Run::new();
        let mut second = Run::new();
        first.ignored = vec!["*".to_owned()];
        second.ignored = vec!["*".to_owned()];
        for (step, op) in ops.iter().enumerate() {
            let _ = first.apply(step, op);
            let _ = second.apply(step, op);
        }
        let difference = first_difference(&state_json(&first), &state_json(&second), "");
        prop_assert!(difference.is_none(), "{}", difference.unwrap_or_default());
    }

    #[test]
    fn saving_and_reloading_does_not_change_what_happens_next(ops in history(), frames in 1u16..1200) {
        let mut run = Run::new();
        run.ignored = vec!["*".to_owned()];
        for (step, op) in ops.iter().enumerate() {
            let _ = run.apply(step, op);
        }
        let mut reloaded = Run::new();
        reloaded.ignored = vec!["*".to_owned()];
        let json = state_json(&run).to_string();
        let (tower, traffic) = load_save(&json).map_err(TestCaseError::fail)?;
        reloaded.tower = tower;
        reloaded.traffic = traffic;
        for _ in 0..frames {
            frame(&mut run.tower, &mut run.traffic, FRAME_SECONDS);
            frame(&mut reloaded.tower, &mut reloaded.traffic, FRAME_SECONDS);
        }
        let difference = first_difference(&state_json(&run), &state_json(&reloaded), "");
        prop_assert!(difference.is_none(), "{}", difference.unwrap_or_default());
    }
}

/// Prints what generated histories exercise. Run with
/// cargo test --release -p simtower-core history_coverage -- --ignored --nocapture
#[test]
#[ignore = "coverage report"]
fn history_coverage() {
    use proptest::strategy::ValueTree;
    let mut runner = proptest::test_runner::TestRunner::deterministic();
    let mut totals = Stats::default();
    let mut with_riders = 0;
    let mut with_people = 0;
    let samples = 200;
    for _ in 0..samples {
        let ops = history().new_tree(&mut runner).unwrap().current();
        let mut run = Run::new();
        run.ignored = vec!["*".to_owned()];
        for (step, op) in ops.iter().enumerate() {
            let _ = run.apply(step, op);
        }
        for (name, (tried, worked)) in &run.stats.applied {
            let entry = totals.applied.entry(name).or_default();
            entry.0 += tried;
            entry.1 += worked;
        }
        totals.frames += run.stats.frames;
        totals.rider_frames += run.stats.rider_frames;
        totals.max_people = totals.max_people.max(run.stats.max_people);
        totals.max_riders = totals.max_riders.max(run.stats.max_riders);
        totals.max_waiting = totals.max_waiting.max(run.stats.max_waiting);
        totals.max_cars = totals.max_cars.max(run.stats.max_cars);
        totals.shafts += run.stats.shafts;
        totals.occupied_tenants += run.stats.occupied_tenants;
        totals.game_minutes += run.stats.game_minutes;
        with_riders += usize::from(run.stats.max_riders > 0);
        with_people += usize::from(run.stats.max_people > 0);
    }
    println!("{samples} histories: {with_people} had people, {with_riders} had riders");
    println!(
        "frames {} (avg {}), game minutes avg {}, rider-frames {}",
        totals.frames,
        totals.frames / samples,
        totals.game_minutes / samples as u64,
        totals.rider_frames
    );
    println!(
        "max people {}, max riders {}, max waiting {}, max cars {}",
        totals.max_people, totals.max_riders, totals.max_waiting, totals.max_cars
    );
    println!(
        "avg shafts {:.1}, avg occupied tenants {:.1}",
        totals.shafts as f32 / samples as f32,
        totals.occupied_tenants as f32 / samples as f32
    );
    for (name, (tried, worked)) in &totals.applied {
        println!("  {name:<14} tried {tried:>5}  took effect {worked:>5}");
    }
}
