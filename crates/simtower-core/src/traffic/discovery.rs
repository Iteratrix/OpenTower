//! Discovery harness (fork-only): drives the simulation through the desktop's frame order and
//! checks cross-structure invariants after every frame. Used by the save-replay test below and
//! by the operation-sequence property tests in `discovery::props`.

use std::collections::{BTreeMap, HashMap, HashSet};

use super::*;
use crate::{DEFAULT_TOWER_WIDTH, Facility, SimulationSpeed, Tower};

mod fuzz;
mod props;
mod reach;

pub(super) const FRAME_SECONDS: f32 = 1.0 / 60.0;

const RIDE_LIMIT: f32 = 300.0;
const WALK_LIMIT: f32 = 240.0;
const STAIRS_LIMIT: f32 = 30.0;
const LEAVING_LIMIT: f32 = 10.0;
const MISSING_SHAFT_WAIT_LIMIT: f32 = 2.0;
const IDLE_BESIDE_WAITER_LIMIT: f32 = 5.0;
const LOADED_CAR_STILL_LIMIT: f32 = 20.0;
/// Game minutes. The longest stay (a condo visit) is 12 game hours.
const PRESENCE_LIMIT_MINUTES: u64 = 2 * 24 * 60;

thread_local! {
    static PEOPLE_CAP: std::cell::Cell<usize> = const { std::cell::Cell::new(MAX_SIMULATED_PEOPLE) };
}

/// The people cap the simulation uses on this thread (see `traffic::people_cap`).
pub(super) fn people_cap() -> usize {
    PEOPLE_CAP.get()
}

/// Lowers the people cap on this thread until dropped, so a small tower reaches the cap.
pub(super) struct PeopleCap(usize);

impl PeopleCap {
    pub fn set(cap: usize) -> Self {
        Self(PEOPLE_CAP.replace(cap))
    }
}

impl Drop for PeopleCap {
    fn drop(&mut self) {
        PEOPLE_CAP.set(self.0);
    }
}

pub(super) fn traffic_scale(speed: SimulationSpeed) -> f32 {
    match speed {
        SimulationSpeed::Paused => 0.0,
        SimulationSpeed::Normal => 1.0,
        SimulationSpeed::Fast => 2.0,
        SimulationSpeed::Triple => 3.0,
        SimulationSpeed::Quintuple => 5.0,
        SimulationSpeed::Tenfold => 10.0,
    }
}

/// One desktop frame in edit view, in `main.rs` order: the economy tick (move-ins gated on
/// reachability, income from observed patronage), then traffic. Desktop-only systems
/// (housekeeping dispatch, hotel service, quarterly evaluation, fires) are not mirrored.
/// Returns the simulated traffic seconds that elapsed.
pub(super) fn frame(tower: &mut Tower, traffic: &mut TrafficSimulation, frame_dt: f32) -> f32 {
    let simulation_dt = frame_dt * traffic_scale(tower.clock.speed);
    let reachable = traffic.reachable_facility_set().clone();
    let weekend = is_weekend(tower.clock);
    let visitors = tower
        .facilities()
        .iter()
        .map(|facility| {
            let patronage = if !facility.is_occupied()
                || !reachable.contains(&facility.id)
                || (!traffic.has_received_visitor(facility.id)
                    && !matches!(
                        facility.kind,
                        FacilityKind::FastFood
                            | FacilityKind::Restaurant
                            | FacilityKind::Shop
                            | FacilityKind::Cinema
                            | FacilityKind::PartyHall
                    )) {
                0
            } else {
                traffic
                    .daily_visitors_at(facility.id)
                    .max(traffic.visitors_at(facility.id) as u32)
                    .max(traffic.dedicated_patronage_at(facility.id, facility.kind, weekend))
            };
            (facility.id, patronage)
        })
        .collect::<Vec<_>>();
    let cars = traffic
        .elevators()
        .iter()
        .map(|shaft| (shaft.id, shaft.kind, shaft.cars.len() as u8))
        .collect::<Vec<_>>();
    let reachable = reachable.into_iter().collect::<Vec<_>>();
    tower.advance_time_with_connected_economy(frame_dt, &visitors, &cars, Some(&reachable));
    traffic.advance_after_sync(tower, simulation_dt);
    simulation_dt
}

/// Loads a save the way the desktop does: validate both halves, widen old sites, then sync.
pub(super) fn load_save(json: &str) -> Result<(Tower, TrafficSimulation), String> {
    #[derive(Deserialize)]
    struct Save {
        tower: Tower,
        traffic: TrafficSimulation,
    }
    let save: Save = serde_json::from_str(json).map_err(|error| error.to_string())?;
    let (mut tower, mut traffic) = (save.tower, save.traffic);
    tower.validate_save_state()?;
    traffic.validate_save_state(&tower)?;
    tower.expand_site_width(DEFAULT_TOWER_WIDTH);
    traffic.sync_with_loaded_tower(&tower);
    Ok((tower, traffic))
}

#[derive(Clone, Debug)]
pub(super) struct Violation {
    pub kind: &'static str,
    pub detail: String,
}

/// Invariant checker with the little history some invariants need (how long a rider has been
/// riding, how long a car has stood idle beside a waiter, ...).
#[derive(Default)]
pub(super) struct Monitor {
    pub elapsed: f32,
    riding: HashMap<u64, f32>,
    walking: HashMap<u64, f32>,
    stairs: HashMap<u64, f32>,
    leaving: HashMap<u64, f32>,
    waiting_for_missing_shaft: HashMap<u64, f32>,
    idle_beside_waiter: HashMap<(u64, usize, u64), f32>,
    car_positions: HashMap<(u64, usize), f32>,
    loaded_car_still: HashMap<(u64, usize), f32>,
    since_route_check: Option<f32>,
    /// Game minute each person was first seen in the tower in their current stay.
    present_since: HashMap<u64, u64>,
    reported: HashSet<(&'static str, u64)>,
    /// Residents whose trip had ended at the previous check, and everyone away then.
    due: HashSet<u64>,
    away: HashSet<u64>,
}

impl Monitor {
    /// Checks every invariant; `dt` is the simulated traffic time since the previous check.
    pub fn check(&mut self, tower: &Tower, traffic: &TrafficSimulation, dt: f32) -> Vec<Violation> {
        self.elapsed += dt;
        let mut found = Vec::new();
        let mut flag = |kind: &'static str, detail: String| found.push(Violation { kind, detail });

        if let Err(error) = traffic.validate_save_state(tower) {
            flag("save validation fails", error);
        }
        if traffic.people.len() > people_cap() {
            flag(
                "people over the cap",
                format!("{} people", traffic.people.len()),
            );
        }

        let people: HashMap<u64, &Person> = traffic
            .people
            .iter()
            .map(|person| (person.id, person))
            .collect();
        let shafts: HashMap<u64, &ElevatorShaft> = traffic
            .elevators
            .iter()
            .map(|shaft| (shaft.id, shaft))
            .collect();

        // Car passenger lists against the people who are riding.
        let mut seated = HashMap::<u64, (u64, usize)>::new();
        for shaft in &traffic.elevators {
            let (Some(&bottom), Some(&top)) =
                (shaft.served_floors.first(), shaft.served_floors.last())
            else {
                flag("shaft serves no floors", format!("shaft {}", shaft.id));
                continue;
            };
            let gaps = (floor_ordinal(bottom)..=floor_ordinal(top))
                .map(floor_from_ordinal)
                .filter(|floor| !shaft.served_floors.contains(floor))
                .collect::<Vec<_>>();
            if !gaps.is_empty() {
                flag(
                    "shaft has a gap",
                    format!(
                        "{:?} x{} serves {bottom}..{top} but not {gaps:?}",
                        shaft.kind, shaft.x
                    ),
                );
            }
            let (low, high) = (
                f32::from(floor_ordinal(bottom)),
                f32::from(floor_ordinal(top)),
            );
            for (index, car) in shaft.cars.iter().enumerate() {
                if car.floor_position < low - 0.01 || car.floor_position > high + 0.01 {
                    flag(
                        "car outside its shaft",
                        format!(
                            "{:?} x{} car {index} at ordinal {:.2}, shaft {bottom}..{top}",
                            shaft.kind, shaft.x, car.floor_position
                        ),
                    );
                }
                if car
                    .target_floor
                    .is_some_and(|floor| !shaft.served_floors.contains(&floor))
                {
                    flag(
                        "car targets a floor its shaft lacks",
                        format!(
                            "{:?} x{} car {index} -> {:?}",
                            shaft.kind, shaft.x, car.target_floor
                        ),
                    );
                }
                if car.passengers.len() > elevator_capacity(shaft.kind) {
                    flag(
                        "car over capacity",
                        format!("{:?} x{} car {index}", shaft.kind, shaft.x),
                    );
                }
                for &id in &car.passengers {
                    if let Some(previous) = seated.insert(id, (shaft.id, index)) {
                        flag(
                            "passenger in two cars",
                            format!("person {id} in {previous:?} and ({}, {index})", shaft.id),
                        );
                    }
                    match people.get(&id) {
                        None => flag(
                            "ghost passenger",
                            format!(
                                "{:?} x{} car {index} carries missing person {id}",
                                shaft.kind, shaft.x
                            ),
                        ),
                        Some(person)
                            if person.activity
                                != (PersonActivity::RidingElevator { shaft_id: shaft.id }) =>
                        {
                            flag(
                                "passenger not riding",
                                format!(
                                    "person {id} listed in {:?} x{} car {index} but {:?}",
                                    shaft.kind, shaft.x, person.activity
                                ),
                            )
                        }
                        Some(_) => {}
                    }
                }
            }
        }

        let mut alive = HashSet::new();
        for person in &traffic.people {
            alive.insert(person.id);
            let id = person.id;
            match person.activity {
                PersonActivity::RidingElevator { shaft_id } => {
                    match shafts.get(&shaft_id) {
                        None => flag(
                            "rider of a missing shaft",
                            format!("person {id} rides {shaft_id}"),
                        ),
                        Some(shaft) => {
                            if seated
                                .get(&id)
                                .is_none_or(|(seated_shaft, _)| *seated_shaft != shaft_id)
                            {
                                flag(
                                    "rider in no car",
                                    format!(
                                        "person {id} rides {:?} x{} but no car lists them",
                                        shaft.kind, shaft.x
                                    ),
                                );
                            }
                            let wanted = desired_elevator_floor(person);
                            if !shaft.served_floors.contains(&wanted) {
                                flag(
                                    "rider bound for a floor the shaft lacks",
                                    format!(
                                        "person {id} on {:?} x{} wants {wanted}",
                                        shaft.kind, shaft.x
                                    ),
                                );
                            }
                        }
                    }
                    let ridden = self.riding.entry(id).or_default();
                    *ridden += dt;
                    if *ridden > RIDE_LIMIT {
                        flag(
                            "rider never arrives",
                            format!("person {id} riding {ridden:.0}s, {:?}", person.route),
                        );
                        *ridden = f32::NEG_INFINITY;
                    }
                }
                PersonActivity::WaitingForElevator { shaft_id } => {
                    if shafts.contains_key(&shaft_id) {
                        self.waiting_for_missing_shaft.remove(&id);
                    } else {
                        let waited = self.waiting_for_missing_shaft.entry(id).or_default();
                        *waited += dt;
                        if *waited > MISSING_SHAFT_WAIT_LIMIT {
                            flag(
                                "waiting for a missing shaft",
                                format!("person {id} waits for {shaft_id}"),
                            );
                            *waited = f32::NEG_INFINITY;
                        }
                    }
                }
                _ => {}
            }
            let mut track = |clock: &mut HashMap<u64, f32>,
                             active: bool,
                             limit: f32,
                             kind: &'static str| {
                if !active {
                    clock.remove(&id);
                    return;
                }
                let spent = clock.entry(id).or_default();
                *spent += dt;
                if *spent > limit {
                    flag(
                        kind,
                        format!(
                            "person {id} {:?} on floor {} at x{:.1} for {:.0}s, route {:?} leg {}",
                            person.activity,
                            person.current_floor,
                            person.x,
                            *spent,
                            person.route,
                            person.route_leg
                        ),
                    );
                    *spent = f32::NEG_INFINITY;
                }
            };
            track(
                &mut self.walking,
                person.activity == PersonActivity::Walking,
                WALK_LIMIT,
                "walker never arrives",
            );
            track(
                &mut self.stairs,
                person.activity == PersonActivity::UsingStairs,
                STAIRS_LIMIT,
                "stuck on stairs",
            );
            track(
                &mut self.leaving,
                person.activity == PersonActivity::LeavingElevatorQueue,
                LEAVING_LIMIT,
                "leaver never leaves",
            );
            if !matches!(person.activity, PersonActivity::RidingElevator { .. }) {
                self.riding.remove(&id);
            }
        }
        for person in &traffic.away {
            if person.activity != PersonActivity::Visiting
                || person.destination_facility_id != 0
                || person.going_home
            {
                flag(
                    "away entry not outside",
                    format!("person {} {:?}", person.id, person.activity),
                );
            }
            if alive.contains(&person.id) {
                flag(
                    "person both away and present",
                    format!("person {}", person.id),
                );
            }
        }
        // Every stay ends: someone in the tower for days holds a slot for good, which a small
        // cap turns into starvation. And residents whose trip has ended come back in ahead of
        // anyone new: nobody new enters while a resident due at the last check is still out.
        let now = tower.clock.absolute_minute();
        let still_due = traffic
            .away
            .iter()
            .filter(|person| person.visit_remaining <= 0.0 && self.due.contains(&person.id))
            .map(|person| person.id)
            .collect::<Vec<_>>();
        if !still_due.is_empty() {
            for newcomer in traffic.people.iter().filter(|person| {
                !self.present_since.contains_key(&person.id) && !self.away.contains(&person.id)
            }) {
                if self.reported.insert(("newcomer", newcomer.id)) {
                    flag(
                        "newcomer ahead of a returning resident",
                        format!(
                            "person {} {:?} bound for {} came in while residents {still_due:?} were due back, {}/{} slots taken",
                            newcomer.id,
                            newcomer.role,
                            newcomer.destination_facility_id,
                            traffic.people.len(),
                            people_cap()
                        ),
                    );
                }
            }
        }
        for person in &traffic.people {
            let since = *self.present_since.entry(person.id).or_insert(now);
            if now - since > PRESENCE_LIMIT_MINUTES && self.reported.insert(("stay", person.id)) {
                flag(
                    "person never leaves",
                    format!(
                        "person {} {:?} {:?} on floor {} for {} game minutes, bound for {} from {}",
                        person.id,
                        person.role,
                        person.activity,
                        person.current_floor,
                        now - since,
                        person.destination_facility_id,
                        person.home_facility_id
                    ),
                );
            }
        }
        self.present_since.retain(|id, _| alive.contains(id));
        self.due = traffic
            .away
            .iter()
            .filter(|person| person.visit_remaining <= 0.0)
            .map(|person| person.id)
            .collect();
        self.away = traffic.away.iter().map(|person| person.id).collect();

        for clock in [
            &mut self.riding,
            &mut self.walking,
            &mut self.stairs,
            &mut self.leaving,
            &mut self.waiting_for_missing_shaft,
        ] {
            clock.retain(|id, _| alive.contains(id));
        }

        // A car standing still with room on a floor where someone waits for its shaft.
        let mut still_beside = HashSet::new();
        for shaft in &traffic.elevators {
            for (index, car) in shaft.cars.iter().enumerate() {
                let previous = self
                    .car_positions
                    .insert((shaft.id, index), car.floor_position);
                let stationary =
                    previous.is_some_and(|position| (position - car.floor_position).abs() < 1e-4);
                let still = self.loaded_car_still.entry((shaft.id, index)).or_default();
                if stationary && !car.passengers.is_empty() {
                    *still += dt;
                    if *still > LOADED_CAR_STILL_LIMIT {
                        flag(
                            "car with riders stands still",
                            format!(
                                "{:?} x{} car {index} at ordinal {:.2} with {}/{} aboard, target {:?}, dir {:?}, dwell {:.1}",
                                shaft.kind,
                                shaft.x,
                                car.floor_position,
                                car.passengers.len(),
                                elevator_capacity(shaft.kind),
                                car.target_floor,
                                car.direction,
                                car.dwell_remaining
                            ),
                        );
                        *still = f32::NEG_INFINITY;
                    }
                } else {
                    *still = 0.0;
                }
                let at_floor = (car.floor_position - car.floor_position.round()).abs() < 0.01;
                if !stationary
                    || !at_floor
                    || car.dwell_remaining > 0.0
                    || car.passengers.len() >= elevator_capacity(shaft.kind)
                {
                    continue;
                }
                let floor = floor_from_ordinal(car.floor_position.round() as i16);
                for person in traffic.people.iter().filter(|person| {
                    person.current_floor == floor
                        && person.activity
                            == (PersonActivity::WaitingForElevator { shaft_id: shaft.id })
                }) {
                    let key = (shaft.id, index, person.id);
                    still_beside.insert(key);
                    let idle = self.idle_beside_waiter.entry(key).or_default();
                    *idle += dt;
                    if *idle > IDLE_BESIDE_WAITER_LIMIT {
                        flag(
                            "idle car beside a waiter",
                            format!(
                                "{:?} x{} car {index} stands at floor {floor} with {}/{} aboard, target {:?}, dir {:?}, while person {} waits {:.0}s",
                                shaft.kind,
                                shaft.x,
                                car.passengers.len(),
                                elevator_capacity(shaft.kind),
                                car.target_floor,
                                car.direction,
                                person.id,
                                person.wait_seconds
                            ),
                        );
                        *idle = f32::NEG_INFINITY;
                    }
                }
            }
        }
        self.idle_beside_waiter
            .retain(|key, _| still_beside.contains(key));

        // Cached lobby routes must still describe something that exists. Routes only change
        // when the tower is edited, so this costly check runs once per simulated minute.
        let since = self.since_route_check.get_or_insert(f32::INFINITY);
        *since += dt;
        if *since < 60.0 {
            return found;
        }
        *since = 0.0;
        let facilities: HashMap<u64, &Facility> = tower
            .facilities()
            .iter()
            .map(|facility| (facility.id, facility))
            .collect();
        for (&facility_id, route) in &traffic.routes_from_lobby {
            let Some(destination) = facilities.get(&facility_id).copied() else {
                flag(
                    "route to a missing facility",
                    format!("facility {facility_id}"),
                );
                continue;
            };
            if let Some(problem) = route_problem(*route, destination, &shafts, tower) {
                flag(
                    "cached route is broken",
                    format!(
                        "{:?} on floor {}: {problem}; {route:?}",
                        destination.kind, destination.position.floor
                    ),
                );
            }
        }
        for id in traffic.reachable_facility_set() {
            if !traffic.routes_from_lobby.contains_key(id) {
                flag("reachable without a route", format!("facility {id}"));
            }
        }
        let (unrouted, cut_off) = reach::Model::new(tower, traffic).audit(traffic);
        for (tenant, verdict) in unrouted {
            if let reach::Verdict::RouterMissed(shape) = verdict {
                flag(
                    "router misses a supported route",
                    format!(
                        "{:?} {} on floor {} at x{}: {shape}",
                        tenant.kind, tenant.id, tenant.position.floor, tenant.position.x
                    ),
                );
            }
        }
        for tenant in cut_off {
            flag(
                "routed tenant is cut off",
                format!(
                    "{:?} {} on floor {} at x{} via {:?}",
                    tenant.kind,
                    tenant.id,
                    tenant.position.floor,
                    tenant.position.x,
                    traffic.routes_from_lobby.get(&tenant.id)
                ),
            );
        }
        found
    }
}

/// An independent, deliberately plain check that a cached lobby route still uses transport
/// that exists and stops where the route needs it. It reads raw shaft floors and walkway
/// facilities rather than the routing helpers.
fn route_problem(
    route: TravelMode,
    destination: &Facility,
    shafts: &HashMap<u64, &ElevatorShaft>,
    tower: &Tower,
) -> Option<String> {
    let serves = |shaft_id: u64, floor: i16| -> Result<(), String> {
        let Some(shaft) = shafts.get(&shaft_id) else {
            return Err(format!("shaft {shaft_id} is gone"));
        };
        if shaft.served_floors.contains(&floor) {
            Ok(())
        } else {
            Err(format!(
                "{:?} x{} doesn't reach floor {floor}",
                shaft.kind, shaft.x
            ))
        }
    };
    let walkway_spans = |x: f32, from: i16, to: i16| -> Result<(), String> {
        let (low, high) = (
            floor_ordinal(from).min(floor_ordinal(to)),
            floor_ordinal(from).max(floor_ordinal(to)),
        );
        for base in low..high {
            let covered = tower.facilities().iter().any(|facility| {
                matches!(
                    facility.kind,
                    FacilityKind::Stairs | FacilityKind::Escalator
                ) && (f32::from(facility.position.x) + f32::from(facility.kind.spec().width) * 0.5
                    - x)
                    .abs()
                    < 0.01
                    && floor_ordinal(facility.position.floor) <= base
                    && base
                        < floor_ordinal(facility.position.floor)
                            + i16::from(facility.stories.max(2))
                            - 1
            });
            if !covered {
                return Err(format!(
                    "no walkway at x{x} covers the flight above ordinal {base}"
                ));
            }
        }
        Ok(())
    };
    let floor = destination.position.floor;
    let result = match route {
        TravelMode::SameFloor => (floor == 1)
            .then_some(())
            .ok_or_else(|| "same-floor route off floor 1".to_owned()),
        TravelMode::Elevator { shaft_id, .. } => serves(shaft_id, 1).and(serves(shaft_id, floor)),
        TravelMode::ElevatorTransfer {
            first_id,
            transfer_floor,
            second_id,
            ..
        } => serves(first_id, 1)
            .and(serves(first_id, transfer_floor))
            .and(serves(second_id, transfer_floor))
            .and(serves(second_id, floor)),
        TravelMode::Stairs { x } => walkway_spans(x, 1, floor),
        TravelMode::ElevatorThenWalkway {
            shaft_id,
            transfer_floor,
            walkway_x,
            ..
        } => serves(shaft_id, 1)
            .and(serves(shaft_id, transfer_floor))
            .and(walkway_spans(walkway_x, transfer_floor, floor)),
        TravelMode::WalkwayThenElevator {
            walkway_x,
            transfer_floor,
            shaft_id,
            ..
        } => walkway_spans(walkway_x, 1, transfer_floor)
            .and(serves(shaft_id, transfer_floor))
            .and(serves(shaft_id, floor)),
    };
    result.err()
}

/// Running tally of violations by kind, keeping the first few details of each.
#[derive(Default)]
pub(super) struct Report {
    pub kinds: BTreeMap<&'static str, (usize, f32, Vec<String>)>,
}

impl Report {
    pub fn add(&mut self, elapsed: f32, violations: Vec<Violation>) {
        for violation in violations {
            let entry = self
                .kinds
                .entry(violation.kind)
                .or_insert((0, elapsed, Vec::new()));
            entry.0 += 1;
            if entry.2.len() < 3 {
                entry
                    .2
                    .push(format!("t={elapsed:.0}s {}", violation.detail));
            }
        }
    }

    pub fn print(&self, label: &str) {
        println!("== {label}: {} violation kinds", self.kinds.len());
        for (kind, (count, first, details)) in &self.kinds {
            println!("  {kind}: {count} (first at {first:.0}s)");
            for detail in details {
                println!("      {detail}");
            }
        }
    }
}

/// Replays real saves for several game days with the invariant battery. Ignored by default;
/// run with DISCOVERY_SAVES=a.smtower:b.smtower DISCOVERY_DAYS=3 DISCOVERY_SPEED=Tenfold
/// cargo test --release -p simtower-core replay_saves -- --ignored --nocapture
#[test]
#[ignore = "needs DISCOVERY_SAVES"]
fn replay_saves() {
    let saves = std::env::var("DISCOVERY_SAVES").expect("DISCOVERY_SAVES");
    let days: f32 = std::env::var("DISCOVERY_DAYS").map_or(3.0, |days| days.parse().unwrap());
    let speed = match std::env::var("DISCOVERY_SPEED").as_deref() {
        Ok("Tenfold") => SimulationSpeed::Tenfold,
        Ok("Fast") => SimulationSpeed::Fast,
        _ => SimulationSpeed::Normal,
    };
    for path in saves.split(':') {
        let json = std::fs::read_to_string(path).expect("read save");
        let (mut tower, mut traffic) = load_save(&json).expect("load save");
        tower.clock.speed = speed;
        // DISCOVERY_CAP replays under a lower people cap; frames run unchecked until the people
        // already in the save drain below it.
        let _cap = std::env::var("DISCOVERY_CAP").ok().map(|cap| {
            let cap = PeopleCap::set(cap.parse().expect("DISCOVERY_CAP"));
            let drain_start = tower.clock.absolute_minute();
            while traffic.people.len() > people_cap() {
                frame(&mut tower, &mut traffic, FRAME_SECONDS);
                assert!(
                    tower.clock.absolute_minute() - drain_start < 24 * 60,
                    "{} people still over the cap after a day",
                    traffic.people.len()
                );
            }
            println!(
                "   drained to {} people in {} game minutes",
                traffic.people.len(),
                tower.clock.absolute_minute() - drain_start
            );
            cap
        });
        let start = tower.clock.absolute_minute();
        let end = start + (days * 24.0 * 60.0) as u64;
        print_unreachable(&tower, &traffic, path);
        let mut monitor = Monitor::default();
        let mut report = Report::default();
        report.add(0.0, monitor.check(&tower, &traffic, 0.0));
        while tower.clock.absolute_minute() < end {
            let dt = frame(&mut tower, &mut traffic, FRAME_SECONDS);
            let violations = monitor.check(&tower, &traffic, dt);
            report.add(monitor.elapsed, violations);
        }
        report.print(&format!("{path} at {speed:?} for {days} days"));
    }
}

/// Lists the tenants the router leaves unreachable, grouped by the oracle's verdict.
fn print_unreachable(tower: &Tower, traffic: &TrafficSimulation, label: &str) {
    let (unrouted, _) = reach::Model::new(tower, traffic).audit(traffic);
    let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (tenant, verdict) in unrouted {
        let place = format!(
            "{:?} {} floor {} x{}",
            tenant.kind, tenant.id, tenant.position.floor, tenant.position.x
        );
        let (group, detail) = match verdict {
            reach::Verdict::RouterMissed(shape) => (
                "router missed a supported route".to_owned(),
                format!("{place}: {shape}"),
            ),
            reach::Verdict::DesignGap { category, path } => (
                format!("design gap: {category}"),
                format!("{place}: {path}"),
            ),
            reach::Verdict::CutOff => ("not connected".to_owned(), place),
        };
        groups.entry(group).or_default().push(detail);
    }
    println!(
        "== {label}: {} unreachable tenants",
        groups.values().map(Vec::len).sum::<usize>()
    );
    for (group, details) in groups {
        println!("  {group}: {}", details.len());
        for detail in details.iter().take(8) {
            println!("      {detail}");
        }
    }
}

/// Finding 1: a car heading home to the caller's floor counts as committed to the call and
/// outranks an empty idle car already standing there.
#[test]
fn finding_homing_car_beats_idle_car_at_the_callers_floor() {
    let mut shaft = ElevatorShaft {
        id: 1,
        kind: FacilityKind::Elevator,
        x: 8,
        served_floors: (1..=30).collect(),
        cars: Vec::new(),
        schedules: [ElevatorSchedule::default(); 12],
        visible: true,
    };
    let car = |floor_position: f32, target_floor, direction| ElevatorCar {
        floor_position,
        home_floor: 1,
        direction,
        target_floor,
        passengers: Vec::new(),
        dwell_remaining: 0.0,
    };
    shaft.cars.push(car(0.0, None, ElevatorDirection::Idle));
    shaft.cars.push(car(24.0, Some(1), ElevatorDirection::Down));
    let waiter = Person {
        id: 7,
        role: PersonRole::Visitor,
        home_facility_id: 0,
        destination_facility_id: 0,
        x: 10.0,
        floor_position: 0.0,
        current_floor: 1,
        mood: PersonMood::Calm,
        activity: PersonActivity::WaitingForElevator { shaft_id: 1 },
        wait_seconds: 0.0,
        origin: Endpoint {
            facility_id: 0,
            x: 0.0,
            floor: 1,
        },
        destination: Endpoint {
            facility_id: 0,
            x: 10.0,
            floor: 12,
        },
        route: TravelMode::Elevator {
            shaft_id: 1,
            x: 10.0,
        },
        going_home: false,
        walking_from_transport: false,
        visit_remaining: 0.0,
        visit_duration: 1.0,
        stair_progress: 0.0,
        route_leg: 0,
    };
    let assignments = assign_waiting_calls(&shaft, &[waiter], 5);
    assert_eq!(
        assignments,
        vec![(1, 0)],
        "the idle car at the lobby should take the call"
    );
}

/// Finding 2: under an ExpressToTop (or ExpressToBottom) schedule, a car with anyone aboard
/// always targets the end floor, so riders bound for any other floor are never let off.
#[test]
fn finding_express_schedule_traps_riders_bound_elsewhere() {
    for (mode, start, wanted) in [
        (ElevatorMode::ExpressToTop, 2.0, 3),
        (ElevatorMode::ExpressToBottom, 7.0, 6),
    ] {
        let mut shaft = ElevatorShaft {
            id: 1,
            kind: FacilityKind::Elevator,
            x: 8,
            served_floors: (1..=10).collect(),
            cars: vec![ElevatorCar {
                floor_position: start,
                home_floor: 1,
                direction: ElevatorDirection::Idle,
                target_floor: None,
                passengers: vec![7],
                dwell_remaining: 0.0,
            }],
            schedules: [ElevatorSchedule {
                mode,
                ..ElevatorSchedule::default()
            }; 12],
            visible: true,
        };
        let mut people = vec![Person {
            id: 7,
            role: PersonRole::Visitor,
            home_facility_id: 0,
            destination_facility_id: 0,
            x: 10.0,
            floor_position: start,
            current_floor: 1,
            mood: PersonMood::Calm,
            activity: PersonActivity::RidingElevator { shaft_id: 1 },
            wait_seconds: 0.0,
            origin: Endpoint {
                facility_id: 0,
                x: 0.0,
                floor: 1,
            },
            destination: Endpoint {
                facility_id: 0,
                x: 10.0,
                floor: wanted,
            },
            route: TravelMode::Elevator {
                shaft_id: 1,
                x: 10.0,
            },
            going_home: false,
            walking_from_transport: false,
            visit_remaining: 0.0,
            visit_duration: 1.0,
            stair_progress: 0.0,
            route_leg: 0,
        }];
        let clock = crate::Clock::default();
        for _ in 0..600 {
            advance_shaft(&mut shaft, &mut people, clock, 0.1);
        }
        assert_eq!(
            people[0].activity,
            PersonActivity::Walking,
            "{mode:?}: rider bound for floor {wanted} still aboard, car at ordinal {:.1}",
            shaft.cars[0].floor_position
        );
    }
}
