use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::{Clock, Facility, FacilityKind, GridPosition, Tower};

pub const ELEVATOR_CAPACITY: usize = 17;
pub const EXPRESS_ELEVATOR_CAPACITY: usize = 34;
pub const MAX_ELEVATOR_CARS: usize = 8;
pub const MAX_SIMULATED_PEOPLE: usize = 160;
pub const MAX_ELEVATOR_QUEUE_PER_FLOOR: usize = 12;
pub const CONCERNED_WAIT_SECONDS: f32 = 10.0;
pub const ANGRY_WAIT_SECONDS: f32 = 20.0;
pub const ABANDON_ELEVATOR_WAIT_SECONDS: f32 = 45.0;
/// The original game treats stairs as short-range transport. Beyond four
/// flights tenants expect an elevator connection.
pub const MAX_STAIR_FLIGHTS: u16 = 4;
/// Escalators are useful for only three consecutive flights in either
/// direction before their route is considered impractical.
pub const MAX_ESCALATOR_FLIGHTS: u16 = 3;
const QUEUE_ABANDON_WALK_SECONDS: f32 = 4.0;
/// Route-score surcharge for changing transport at a lobby, between two elevators or between
/// an elevator and stairs or an escalator.
const TRANSFER_PENALTY: f32 = 30.0;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PersonMood {
    Calm,
    Concerned,
    Angry,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PersonActivity {
    Walking,
    WaitingForElevator { shaft_id: u64 },
    RidingElevator { shaft_id: u64 },
    UsingStairs,
    Visiting,
    LeavingElevatorQueue,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ElevatorDirection {
    Idle,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ElevatorMode {
    Local,
    ExpressToTop,
    ExpressToBottom,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ElevatorSchedule {
    pub mode: ElevatorMode,
    pub waiting_car_response: u8,
    pub departure_seconds: u8,
}

impl Default for ElevatorSchedule {
    fn default() -> Self {
        Self {
            mode: ElevatorMode::Local,
            waiting_car_response: 5,
            departure_seconds: 0,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ElevatorCar {
    pub floor_position: f32,
    pub home_floor: i16,
    pub direction: ElevatorDirection,
    pub target_floor: Option<i16>,
    pub passengers: Vec<u64>,
    dwell_remaining: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ElevatorShaft {
    pub id: u64,
    pub kind: FacilityKind,
    pub x: u16,
    pub served_floors: Vec<i16>,
    pub cars: Vec<ElevatorCar>,
    pub schedules: [ElevatorSchedule; 12],
    pub visible: bool,
}

impl ElevatorShaft {
    pub fn current_schedule(&self, clock: Clock) -> ElevatorSchedule {
        self.schedules[schedule_index(clock)]
    }

    pub fn current_schedule_mut(&mut self, clock: Clock) -> &mut ElevatorSchedule {
        &mut self.schedules[schedule_index(clock)]
    }

    pub fn waiting_count(&self, people: &[Person]) -> usize {
        people
            .iter()
            .filter(|person| {
                matches!(
                    person.activity,
                    PersonActivity::WaitingForElevator { shaft_id } if shaft_id == self.id
                )
            })
            .count()
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Person {
    pub id: u64,
    /// The original simulation distinguishes residents and staff by role.
    /// Keeping that identity on the lightweight active traveler lets the UI
    /// select the correct artwork without creating a permanent object for
    /// every member of a large tower's population.
    #[serde(default)]
    pub role: PersonRole,
    pub home_facility_id: u64,
    pub destination_facility_id: u64,
    pub x: f32,
    /// Continuous vertical position in display-floor ordinals. B1 is -1,
    /// floor 1 is 0, floor 2 is 1, and so on.
    pub floor_position: f32,
    pub current_floor: i16,
    pub mood: PersonMood,
    pub activity: PersonActivity,
    pub wait_seconds: f32,
    origin: Endpoint,
    destination: Endpoint,
    route: TravelMode,
    going_home: bool,
    walking_from_transport: bool,
    visit_remaining: f32,
    visit_duration: f32,
    stair_progress: f32,
    route_leg: u8,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum PersonRole {
    #[default]
    Visitor,
    Salesman,
    Woman,
    Child,
    Security,
    WomanWithChild,
    Homebody,
    Housekeeper,
}

impl PersonRole {
    const fn is_staff(self) -> bool {
        matches!(self, Self::Security | Self::Housekeeper)
    }
}

impl Person {
    pub const fn is_returning_home(&self) -> bool {
        self.going_home
    }

    /// Whether the person has reached the street on an outside trip that has not ended yet.
    fn is_away(&self) -> bool {
        self.activity == PersonActivity::Visiting
            && self.destination_facility_id == 0
            && !self.going_home
            && self.visit_remaining > 0.0
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
struct Endpoint {
    facility_id: u64,
    x: f32,
    floor: i16,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
enum TravelMode {
    SameFloor,
    Elevator {
        shaft_id: u64,
        x: f32,
    },
    ElevatorTransfer {
        first_id: u64,
        first_x: f32,
        transfer_floor: i16,
        second_id: u64,
        second_x: f32,
    },
    Stairs {
        x: f32,
    },
    /// An elevator ride to a lobby floor, then stairs or an escalator from that lobby to the
    /// destination floor.
    ElevatorThenWalkway {
        shaft_id: u64,
        shaft_x: f32,
        transfer_floor: i16,
        walkway_x: f32,
    },
    /// Stairs or an escalator to a lobby floor, then an elevator ride from that lobby to the
    /// destination floor. The reverse of [`TravelMode::ElevatorThenWalkway`].
    WalkwayThenElevator {
        walkway_x: f32,
        transfer_floor: i16,
        shaft_id: u64,
        shaft_x: f32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ElevatorAccess {
    Public,
    Housekeeping,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TopologySync {
    Full,
    Additive,
    RemovedFloor(GridPosition),
    RemovedWalkway {
        kind: FacilityKind,
        x: u16,
        floor: i16,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TrafficSimulation {
    people: Vec<Person>,
    /// Residents out of the tower on an outside trip (at work, at school, or out for a service
    /// the tower lacks), in the order they left. They are not in the tower, so they hold no
    /// `MAX_SIMULATED_PEOPLE` slot until they come back in.
    #[serde(default)]
    away: Vec<Person>,
    elevators: Vec<ElevatorShaft>,
    arrived_facilities: HashSet<u64>,
    #[serde(default)]
    reachable_facilities: HashSet<u64>,
    #[serde(default)]
    routes_from_lobby: HashMap<u64, TravelMode>,
    #[serde(default)]
    connected_parking: HashSet<u64>,
    #[serde(default)]
    topology_signature: u64,
    #[serde(default)]
    known_destinations: HashSet<u64>,
    #[serde(default)]
    current_visitors: HashMap<u64, usize>,
    #[serde(default)]
    daily_visits: HashMap<u64, u32>,
    #[serde(default)]
    patronage_day: u32,
    /// One compact day stamp per condo/role replaces persistent resident
    /// agents. Only residents who are currently making a trip consume one of
    /// the capped detailed-person slots.
    #[serde(default)]
    #[serde(alias = "resident_trip_days")]
    scheduled_trip_days: HashMap<u64, u32>,
    /// Long-lived, low-cost commercial demand. This grows independently of
    /// the capped set of detailed people so a large tower does not lose its
    /// economy merely because all visible-agent slots are occupied.
    #[serde(default)]
    commercial_patronage: HashMap<u64, u32>,
    /// Edge-triggered status-bar notification for an office parking request
    /// that could not reserve a connected free space.
    #[serde(default)]
    parking_demand_warning: bool,
    #[serde(default)]
    last_resident_dispatch_bucket: u64,
    #[serde(default)]
    scheduled_home_cursor: usize,
    next_person_id: u64,
    spawn_accumulator: f32,
    next_spawn_seconds: f32,
}

impl Default for TrafficSimulation {
    fn default() -> Self {
        Self {
            people: Vec::new(),
            away: Vec::new(),
            elevators: Vec::new(),
            arrived_facilities: HashSet::new(),
            reachable_facilities: HashSet::new(),
            routes_from_lobby: HashMap::new(),
            connected_parking: HashSet::new(),
            topology_signature: 0,
            known_destinations: HashSet::new(),
            current_visitors: HashMap::new(),
            daily_visits: HashMap::new(),
            patronage_day: 0,
            scheduled_trip_days: HashMap::new(),
            commercial_patronage: HashMap::new(),
            parking_demand_warning: false,
            last_resident_dispatch_bucket: 0,
            scheduled_home_cursor: 0,
            next_person_id: 1,
            spawn_accumulator: 0.0,
            next_spawn_seconds: 1.0,
        }
    }
}

impl TrafficSimulation {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn people(&self) -> &[Person] {
        &self.people
    }

    /// Residents currently out of the tower on an outside trip. They come back into
    /// [`Self::people`] when the trip ends.
    pub fn away_people(&self) -> &[Person] {
        &self.away
    }

    pub fn take_parking_demand_warning(&mut self) -> bool {
        std::mem::take(&mut self.parking_demand_warning)
    }

    pub fn elevators(&self) -> &[ElevatorShaft] {
        &self.elevators
    }

    pub fn validate_save_state(&self, tower: &Tower) -> Result<(), String> {
        if self.people.len() > MAX_SIMULATED_PEOPLE
            || !self.spawn_accumulator.is_finite()
            || self.spawn_accumulator < 0.0
            || !self.next_spawn_seconds.is_finite()
            || self.next_spawn_seconds <= 0.0
        {
            return Err("save contains invalid traffic timing data".to_owned());
        }
        let facility_ids = tower
            .facilities()
            .iter()
            .map(|facility| facility.id)
            .collect::<HashSet<_>>();
        let shaft_ids = self
            .elevators
            .iter()
            .map(|shaft| shaft.id)
            .collect::<HashSet<_>>();
        for shaft in &self.elevators {
            if !matches!(
                shaft.kind,
                FacilityKind::Elevator
                    | FacilityKind::ServiceElevator
                    | FacilityKind::ExpressElevator
            ) || shaft.cars.is_empty()
                || shaft.cars.len() > MAX_ELEVATOR_CARS
                || shaft.x >= tower.width()
                || shaft.served_floors.is_empty()
                || shaft
                    .served_floors
                    .iter()
                    .any(|floor| !(crate::MIN_FLOOR..=crate::MAX_FLOOR).contains(floor))
                || shaft.cars.iter().any(|car| {
                    !car.floor_position.is_finite()
                        || !shaft.served_floors.contains(&car.home_floor)
                })
            {
                return Err("save contains invalid elevator state".to_owned());
            }
        }
        let mut person_ids = HashSet::new();
        let everyone = self
            .people
            .iter()
            .map(|person| (person, false))
            .chain(self.away.iter().map(|person| (person, true)));
        for (person, listed_away) in everyone {
            if person.id == 0
                || (listed_away
                    && (person.activity != PersonActivity::Visiting
                        || person.destination_facility_id != 0
                        || person.going_home))
                || !person_ids.insert(person.id)
                || !person.x.is_finite()
                || !person.floor_position.is_finite()
                || !person.wait_seconds.is_finite()
                || (person.home_facility_id != 0
                    && !facility_ids.contains(&person.home_facility_id))
                || (person.destination_facility_id != 0
                    && !facility_ids.contains(&person.destination_facility_id))
                || match person.activity {
                    PersonActivity::WaitingForElevator { shaft_id }
                    | PersonActivity::RidingElevator { shaft_id } => !shaft_ids.contains(&shaft_id),
                    _ => false,
                }
            {
                return Err("save contains invalid person state".to_owned());
            }
        }
        Ok(())
    }

    /// Number of simulated people who have actually reached this facility and
    /// are currently inside it. People walking toward it, waiting for
    /// transport, or riding an elevator are deliberately not counted.
    pub fn visitors_at(&self, facility_id: u64) -> usize {
        self.current_visitors
            .get(&facility_id)
            .copied()
            .unwrap_or(0)
    }

    /// True after at least one simulated person has completed the journey
    /// from outside, through a first-floor lobby, to this tenant.
    pub fn has_received_visitor(&self, facility_id: u64) -> bool {
        self.arrived_facilities.contains(&facility_id)
    }

    /// People who completed a trip into this tenant during the current game
    /// day. Economic settlement uses daily patronage, while `visitors_at`
    /// remains the number physically inside for crowd artwork.
    pub fn daily_visitors_at(&self, facility_id: u64) -> u32 {
        self.daily_visits.get(&facility_id).copied().unwrap_or(0)
    }

    /// Stable patronage generated by a commercial tenant's own advertising
    /// and reputation. Weekend multipliers match the original traffic study;
    /// detailed travelers are layered on top by the desktop economy.
    pub fn dedicated_patronage_at(
        &self,
        facility_id: u64,
        kind: FacilityKind,
        weekend: bool,
    ) -> u32 {
        let base = self
            .commercial_patronage
            .get(&facility_id)
            .copied()
            .unwrap_or_else(|| commercial_patronage_initial(kind));
        match kind {
            FacilityKind::FastFood | FacilityKind::Restaurant if weekend => {
                base.saturating_mul(48).div_ceil(35).min(48)
            }
            FacilityKind::Shop if weekend => base.saturating_mul(30).div_ceil(25).min(30),
            _ => base,
        }
    }

    /// Starts a new cinema program. New releases draw a large opening crowd,
    /// then settle toward the cinema's twenty-person baseline over time.
    pub fn change_cinema_program(&mut self, facility_id: u64) -> bool {
        self.commercial_patronage.insert(facility_id, 120);
        true
    }

    /// Evaluation penalty for the final walk between transport and a tenant.
    /// Express service is exempt because its purpose is to bridge large tower
    /// sections; ordinary tenants progressively dislike long corridors.
    pub fn access_distance_penalty(&self, tower: &Tower, facility_id: u64) -> u8 {
        let Some(facility) = tower
            .facilities()
            .iter()
            .find(|facility| facility.id == facility_id)
        else {
            return 0;
        };
        let Some(route) = self.routes_from_lobby.get(&facility_id).copied() else {
            return 0;
        };
        let uses_express = match route {
            TravelMode::Elevator { shaft_id, .. }
            | TravelMode::ElevatorThenWalkway { shaft_id, .. }
            | TravelMode::WalkwayThenElevator { shaft_id, .. } => self
                .elevators
                .iter()
                .any(|shaft| shaft.id == shaft_id && shaft.kind == FacilityKind::ExpressElevator),
            TravelMode::ElevatorTransfer {
                first_id,
                second_id,
                ..
            } => self.elevators.iter().any(|shaft| {
                (shaft.id == first_id || shaft.id == second_id)
                    && shaft.kind == FacilityKind::ExpressElevator
            }),
            _ => false,
        };
        if uses_express {
            return 0;
        }

        let destination_x = endpoint(facility).x;
        let transport_x = match route {
            TravelMode::SameFloor => tower
                .facilities()
                .iter()
                .filter(|candidate| {
                    candidate.kind == FacilityKind::Lobby
                        && candidate.position.floor == facility.position.floor
                })
                .map(|candidate| endpoint(candidate).x)
                .min_by(|left, right| {
                    (destination_x - *left)
                        .abs()
                        .total_cmp(&(destination_x - *right).abs())
                }),
            route => final_transport_x(route),
        };
        let Some(transport_x) = transport_x else {
            return 0;
        };
        match (destination_x - transport_x).abs() {
            distance if distance > 32.0 => 24,
            distance if distance > 24.0 => 16,
            distance if distance > 16.0 => 8,
            _ => 0,
        }
    }

    /// Sends everyone currently inside a facility back toward the entrance.
    /// Emergency events call this every simulation tick, which also prevents
    /// a late arrival from remaining inside while the facility is unsafe.
    pub fn evacuate_facility(&mut self, facility_id: u64) {
        for person in self.people.iter_mut().filter(|person| {
            person.destination_facility_id == facility_id
                && !person.going_home
                && person.activity == PersonActivity::Visiting
        }) {
            person.going_home = true;
            person.current_floor = person.destination.floor;
            person.floor_position = f32::from(floor_ordinal(person.current_floor));
            person.route = reverse_route(person.route);
            person.route_leg = 0;
            person.walking_from_transport = person.route == TravelMode::SameFloor;
            person.activity = PersonActivity::Walking;
            person.visit_remaining = 0.0;
        }
        self.refresh_current_visitors();
    }

    /// Adds a distinguished outside visitor to one specific tenant while
    /// still using the same route, elevator queues, stress, and exit logic as
    /// every ordinary Sim. The desktop uses this for the four-star VIP visit.
    pub fn spawn_special_visitor(
        &mut self,
        tower: &Tower,
        facility_id: u64,
        visit_seconds: f32,
    ) -> Option<u64> {
        if self.people.len() >= MAX_SIMULATED_PEOPLE {
            return None;
        }
        let facility = tower
            .facilities()
            .iter()
            .find(|facility| facility.id == facility_id && facility.is_occupied())?;
        let route = self.routes_from_lobby.get(&facility_id).copied()?;
        let seed = mix64(self.next_person_id ^ (u64::from(tower.clock.day) << 32));
        let origin = Endpoint {
            facility_id: 0,
            x: exterior_spawn_x(tower, seed),
            floor: 1,
        };
        let destination = endpoint(facility);
        let person_id = self.next_person_id;
        let mut person = Person {
            id: person_id,
            role: PersonRole::Visitor,
            home_facility_id: 0,
            destination_facility_id: facility_id,
            x: origin.x,
            floor_position: f32::from(floor_ordinal(1)),
            current_floor: 1,
            mood: PersonMood::Calm,
            activity: PersonActivity::Walking,
            wait_seconds: 0.0,
            origin,
            destination,
            route,
            going_home: false,
            walking_from_transport: route == TravelMode::SameFloor,
            visit_remaining: 0.0,
            visit_duration: visit_seconds.max(1.0),
            stair_progress: 0.0,
            route_leg: 0,
        };
        start_trip(&mut person);
        self.next_person_id += 1;
        self.people.push(person);
        Some(person_id)
    }

    /// Tenant ids having a complete pedestrian/transport route from a
    /// first-floor lobby. The economy uses this to hold disconnected spaces
    /// vacant even after their normal move-in delay expires.
    pub fn reachable_facility_ids(&self, tower: &Tower) -> Vec<u64> {
        debug_assert_eq!(self.topology_signature, tower_topology_signature(tower));
        self.reachable_facilities.iter().copied().collect()
    }

    /// Cached tenant reachability. `sync_with_tower` refreshes this only when
    /// floors or transport topology change; querying it is constant time.
    pub fn reachable_facility_set(&self) -> &HashSet<u64> {
        &self.reachable_facilities
    }

    /// Whether a housekeeper can walk from a housekeeping office to a room.
    /// Cross-floor routes deliberately consider service elevators only; this
    /// keeps housekeeping staff out of passenger lifts and escalators.
    pub fn housekeeping_route_available(
        &self,
        tower: &Tower,
        housekeeping_id: u64,
        room_id: u64,
    ) -> bool {
        let Some(origin) = tower.facilities().iter().find(|facility| {
            facility.id == housekeeping_id && facility.kind == FacilityKind::Housekeeping
        }) else {
            return false;
        };
        let Some(destination) = tower.facilities().iter().find(|facility| {
            facility.id == room_id
                && matches!(
                    facility.kind,
                    FacilityKind::HotelSingle | FacilityKind::HotelTwin | FacilityKind::HotelSuite
                )
        }) else {
            return false;
        };
        choose_route_scored_for(
            endpoint(origin),
            endpoint(destination),
            &self.elevators,
            tower.facilities(),
            tower.floors(),
            ElevatorAccess::Housekeeping,
        )
        .is_some()
    }

    /// Dispatches one of the six workers assigned to a housekeeping office.
    /// The worker follows the same walking and elevator state machine as every
    /// other person, but cross-floor trips are restricted to service lifts.
    pub fn dispatch_housekeeper(
        &mut self,
        tower: &Tower,
        housekeeping_id: u64,
        room_id: u64,
        cleaning_seconds: f32,
    ) -> Option<u64> {
        if self.people.len() >= MAX_SIMULATED_PEOPLE
            || self
                .people
                .iter()
                .filter(|person| person.home_facility_id == housekeeping_id)
                .count()
                >= 6
            || self.people.iter().any(|person| {
                person.destination_facility_id == room_id && person.home_facility_id != 0
            })
        {
            return None;
        }
        let origin = tower.facilities().iter().find(|facility| {
            facility.id == housekeeping_id && facility.kind == FacilityKind::Housekeeping
        })?;
        let destination = tower.facilities().iter().find(|facility| {
            facility.id == room_id
                && matches!(
                    facility.kind,
                    FacilityKind::HotelSingle | FacilityKind::HotelTwin | FacilityKind::HotelSuite
                )
        })?;
        let origin = endpoint(origin);
        let destination = endpoint(destination);
        let route = choose_route_scored_for(
            origin,
            destination,
            &self.elevators,
            tower.facilities(),
            tower.floors(),
            ElevatorAccess::Housekeeping,
        )?
        .0;
        let person_id = self.next_person_id;
        let mut person = Person {
            id: person_id,
            role: PersonRole::Housekeeper,
            home_facility_id: housekeeping_id,
            destination_facility_id: room_id,
            x: origin.x,
            floor_position: f32::from(floor_ordinal(origin.floor)),
            current_floor: origin.floor,
            mood: PersonMood::Calm,
            activity: PersonActivity::Walking,
            wait_seconds: 0.0,
            origin,
            destination,
            route,
            going_home: false,
            walking_from_transport: route == TravelMode::SameFloor,
            visit_remaining: 0.0,
            visit_duration: cleaning_seconds.max(1.0),
            stair_progress: 0.0,
            route_leg: 0,
        };
        start_trip(&mut person);
        self.next_person_id += 1;
        self.people.push(person);
        Some(person_id)
    }

    pub fn elevator_mut(&mut self, shaft_id: u64) -> Option<&mut ElevatorShaft> {
        self.elevators.iter_mut().find(|shaft| shaft.id == shaft_id)
    }

    pub fn elevator_at(&self, kind: FacilityKind, x: u16) -> Option<&ElevatorShaft> {
        self.elevators
            .iter()
            .find(|shaft| shaft.kind == kind && shaft.x == x)
    }

    pub fn elevator_can_stop_at(&self, tower: &Tower, shaft_id: u64, floor: i16) -> bool {
        self.elevators
            .iter()
            .find(|shaft| shaft.id == shaft_id)
            .is_some_and(|shaft| shaft_stops_at(shaft, floor, tower.facilities()))
    }

    pub fn add_elevator_car(&mut self, tower: &Tower, shaft_id: u64, home_floor: i16) -> bool {
        if !self.elevator_can_stop_at(tower, shaft_id, home_floor) {
            return false;
        }
        let Some(shaft) = self.elevator_mut(shaft_id) else {
            return false;
        };
        if shaft.cars.len() >= MAX_ELEVATOR_CARS {
            return false;
        }
        shaft.cars.push(ElevatorCar {
            floor_position: f32::from(floor_ordinal(home_floor)),
            home_floor,
            direction: ElevatorDirection::Idle,
            target_floor: None,
            passengers: Vec::new(),
            dwell_remaining: 0.0,
        });
        true
    }

    pub fn sync_with_tower(&mut self, tower: &Tower) {
        self.sync_with_tower_internal(tower, TopologySync::Full);
    }

    /// Synchronizes after construction that only adds route infrastructure.
    /// Existing routes remain valid, so only newly placed or previously
    /// unreachable destinations need pathfinding. Destructive edits continue
    /// to use `sync_with_tower` and rebuild every route.
    pub fn sync_with_tower_after_addition(&mut self, tower: &Tower) {
        self.sync_with_tower_internal(tower, TopologySync::Additive);
    }

    /// Synchronizes after removing one facility. A stair or escalator flight
    /// can invalidate only routes that use its column and cross that flight,
    /// so preserve every unrelated cached tenant route instead of rebuilding
    /// the entire tower synchronously on the demolition click.
    pub fn sync_with_tower_after_removal(&mut self, tower: &Tower, removed: &Facility) {
        let mode = if matches!(removed.kind, FacilityKind::Stairs | FacilityKind::Escalator) {
            TopologySync::RemovedWalkway {
                kind: removed.kind,
                x: removed.position.x,
                floor: removed.position.floor,
            }
        } else {
            TopologySync::Full
        };
        self.sync_with_tower_internal(tower, mode);
    }

    /// Synchronizes after removing one structural floor cell. Floor removal
    /// cannot create a new route and does not alter elevator shafts, so only
    /// destinations whose final horizontal walk crossed that cell need fresh
    /// pathfinding.
    pub fn sync_with_tower_after_floor_removal(&mut self, tower: &Tower, removed: GridPosition) {
        self.sync_with_tower_internal(tower, TopologySync::RemovedFloor(removed));
    }

    fn sync_with_tower_internal(&mut self, tower: &Tower, mode: TopologySync) {
        let signature = tower_topology_signature(tower);
        let destinations = tower
            .facilities()
            .iter()
            .filter(|facility| facility.kind.has_tenant_occupancy())
            .collect::<Vec<_>>();
        let destination_ids = destinations
            .iter()
            .map(|facility| facility.id)
            .collect::<HashSet<_>>();
        let topology_changed = self.topology_signature != signature;
        let destinations_changed = self.known_destinations != destination_ids;
        if !topology_changed && !destinations_changed {
            normalize_elevator_home_floors(&mut self.elevators, tower.facilities());
            release_departed_passengers(&mut self.elevators, &self.people);
            return;
        }

        if topology_changed && !matches!(mode, TopologySync::RemovedFloor(_)) {
            let mut groups: Vec<(FacilityKind, u16, u64, Vec<i16>)> = Vec::new();
            for facility in tower.facilities().iter().filter(|facility| {
                matches!(
                    facility.kind,
                    FacilityKind::Elevator
                        | FacilityKind::ServiceElevator
                        | FacilityKind::ExpressElevator
                )
            }) {
                if let Some(group) = groups
                    .iter_mut()
                    .find(|group| group.0 == facility.kind && group.1 == facility.position.x)
                {
                    group.2 = group.2.min(facility.id);
                    if !group.3.contains(&facility.position.floor) {
                        group.3.push(facility.position.floor);
                    }
                } else {
                    groups.push((
                        facility.kind,
                        facility.position.x,
                        facility.id,
                        vec![facility.position.floor],
                    ));
                }
            }

            let mut synchronized = Vec::with_capacity(groups.len());
            let mut renumbered = HashMap::new();
            for (kind, x, id, mut served_floors) in groups {
                served_floors.sort_by_key(|floor| floor_ordinal(*floor));
                if let Some(mut existing) = self
                    .elevators
                    .iter()
                    .find(|shaft| shaft.kind == kind && shaft.x == x)
                    .cloned()
                {
                    if existing.id != id {
                        renumbered.insert(existing.id, id);
                    }
                    existing.id = id;
                    existing.served_floors = served_floors;
                    let landing_floors = landing_floors(
                        existing.kind,
                        existing.x,
                        &existing.served_floors,
                        tower.facilities(),
                    );
                    for car in &mut existing.cars {
                        if !landing_floors.contains(&car.home_floor) {
                            car.home_floor = default_home_floor(&landing_floors);
                        }
                    }
                    synchronized.push(existing);
                } else {
                    let landing_floors =
                        landing_floors(kind, x, &served_floors, tower.facilities());
                    let home_floor = default_home_floor(&landing_floors);
                    synchronized.push(ElevatorShaft {
                        id,
                        kind,
                        x,
                        served_floors,
                        cars: vec![ElevatorCar {
                            floor_position: f32::from(floor_ordinal(home_floor)),
                            home_floor,
                            direction: ElevatorDirection::Idle,
                            target_floor: None,
                            passengers: Vec::new(),
                            dwell_remaining: 0.0,
                        }],
                        schedules: [ElevatorSchedule::default(); 12],
                        visible: true,
                    });
                }
            }
            self.elevators = synchronized;
            settle_people_after_shaft_changes(&mut self.elevators, &mut self.people, &renumbered);
            for person in &mut self.away {
                renumber_person_shafts(person, &renumbered);
            }
        }

        if topology_changed {
            self.connected_parking = connected_parking_spaces(tower.facilities());
        }

        if topology_changed && mode == TopologySync::Full {
            self.routes_from_lobby = destinations
                .iter()
                .filter_map(|facility| {
                    route_from_first_floor_lobby(
                        endpoint(facility),
                        &self.elevators,
                        tower.facilities(),
                        tower.floors(),
                    )
                    .map(|route| (facility.id, route))
                })
                .collect();
        } else {
            // Tenant placement does not change corridors or transport, and
            // additive construction cannot invalidate a working path. A
            // removed walkway invalidates only routes that cross that exact
            // flight. Preserve everything else and retry only new, affected,
            // or previously unreachable destinations.
            self.routes_from_lobby
                .retain(|facility_id, _| destination_ids.contains(facility_id));
            let mut affected = destinations
                .iter()
                .filter(|facility| !self.known_destinations.contains(&facility.id))
                .map(|facility| facility.id)
                .collect::<HashSet<_>>();
            match mode {
                TopologySync::Additive => {
                    // Additions may make any previously unreachable tenant
                    // reachable, so retry only the entries absent from cache.
                    affected.extend(
                        destinations
                            .iter()
                            .filter(|facility| !self.routes_from_lobby.contains_key(&facility.id))
                            .map(|facility| facility.id),
                    );
                }
                TopologySync::RemovedFloor(removed) => {
                    affected.extend(destinations.iter().filter_map(|facility| {
                        self.routes_from_lobby
                            .get(&facility.id)
                            .is_some_and(|route| {
                                route_uses_removed_floor(*route, facility, removed)
                            })
                            .then_some(facility.id)
                    }));
                }
                TopologySync::RemovedWalkway { kind, x, floor } => {
                    affected.extend(destinations.iter().filter_map(|facility| {
                        self.routes_from_lobby
                            .get(&facility.id)
                            .is_some_and(|route| {
                                route_uses_removed_walkway(
                                    *route,
                                    facility.position.floor,
                                    kind,
                                    x,
                                    floor,
                                )
                            })
                            .then_some(facility.id)
                    }));
                }
                TopologySync::Full => {}
            }
            self.routes_from_lobby
                .retain(|facility_id, _| !affected.contains(facility_id));
            for facility in &destinations {
                if !affected.contains(&facility.id) {
                    continue;
                }
                if let Some(route) = route_from_first_floor_lobby(
                    endpoint(facility),
                    &self.elevators,
                    tower.facilities(),
                    tower.floors(),
                ) {
                    self.routes_from_lobby.insert(facility.id, route);
                }
            }
        }
        self.reachable_facilities = self.routes_from_lobby.keys().copied().collect();
        self.topology_signature = signature;
        self.known_destinations = destination_ids;

        let live_facilities: HashSet<u64> = tower
            .facilities()
            .iter()
            .map(|facility| facility.id)
            .collect();
        let still_placed = |person: &Person| {
            (person.home_facility_id == 0 || live_facilities.contains(&person.home_facility_id))
                && (person.destination_facility_id == 0
                    || live_facilities.contains(&person.destination_facility_id))
        };
        self.people.retain(still_placed);
        self.away.retain(still_placed);
        release_departed_passengers(&mut self.elevators, &self.people);
        self.scheduled_trip_days
            .retain(|key, _| live_facilities.contains(&(key >> 5)));
        self.commercial_patronage
            .retain(|facility_id, _| live_facilities.contains(facility_id));
        for facility in tower.facilities().iter().filter(|facility| {
            facility.is_occupied() && commercial_patronage_target(facility.kind).is_some()
        }) {
            self.commercial_patronage
                .entry(facility.id)
                .or_insert_with(|| commercial_patronage_initial(facility.kind));
        }
        self.arrived_facilities
            .retain(|facility_id| live_facilities.contains(facility_id));
        // Condominiums represent residents who already live in the tower;
        // they do not need a generic outside visitor to validate occupancy.
        self.arrived_facilities.extend(
            tower
                .facilities()
                .iter()
                .filter(|facility| {
                    facility.kind == FacilityKind::Condo
                        && facility.is_occupied()
                        && self.reachable_facilities.contains(&facility.id)
                })
                .map(|facility| facility.id),
        );
        self.daily_visits
            .retain(|facility_id, _| live_facilities.contains(facility_id));
        self.refresh_current_visitors();
    }

    pub fn advance(&mut self, tower: &Tower, elapsed_seconds: f32) {
        if elapsed_seconds <= 0.0 {
            return;
        }
        self.sync_with_tower(tower);
        self.advance_after_sync(tower, elapsed_seconds);
    }

    /// Advances traffic after the caller has already synchronized tower
    /// topology for this frame. The desktop needs reachability before its
    /// economy tick, so this avoids hashing and checking the same tower twice.
    pub fn advance_after_sync(&mut self, tower: &Tower, elapsed_seconds: f32) {
        if elapsed_seconds <= 0.0 {
            return;
        }
        if self.patronage_day != tower.clock.day {
            self.patronage_day = tower.clock.day;
            self.daily_visits.clear();
            self.advance_commercial_patronage(tower);
        }
        self.dispatch_scheduled_trips(tower);
        self.spawn_accumulator += elapsed_seconds;
        let population_limit = (usize::from(tower.star_rating()) * 32).min(MAX_SIMULATED_PEOPLE);
        while self.spawn_accumulator >= self.next_spawn_seconds
            && self.people.len() < population_limit
        {
            self.spawn_accumulator -= self.next_spawn_seconds;
            self.next_spawn_seconds = next_spawn_interval(tower.star_rating(), self.next_person_id);
            if !self.spawn_person(tower) {
                break;
            }
        }

        let previously_visiting = self
            .people
            .iter()
            .filter(|person| !person.going_home && person.activity == PersonActivity::Visiting)
            .map(|person| person.id)
            .collect::<HashSet<_>>();
        advance_people(&mut self.people, elapsed_seconds);
        for person in self.people.iter().filter(|person| {
            person.destination_facility_id != 0
                && !person.role.is_staff()
                && !person.going_home
                && person.activity == PersonActivity::Visiting
                && !previously_visiting.contains(&person.id)
        }) {
            *self
                .daily_visits
                .entry(person.destination_facility_id)
                .or_default() += 1;
        }
        abandon_unservable_elevator_waits(&mut self.people, &self.elevators);
        enforce_elevator_queue_limits(&mut self.people);
        self.arrived_facilities.extend(
            self.people
                .iter()
                .filter(|person| {
                    person.destination_facility_id != 0
                        && !person.role.is_staff()
                        && !person.going_home
                        && person.activity == PersonActivity::Visiting
                })
                .map(|person| person.destination_facility_id),
        );
        for shaft in &mut self.elevators {
            advance_shaft(shaft, &mut self.people, tower.clock, elapsed_seconds);
        }
        self.people.retain(|person| {
            !(person.visit_remaining <= 0.0
                && (person.activity == PersonActivity::LeavingElevatorQueue
                    || (person.going_home && person.activity == PersonActivity::Visiting)))
        });
        self.bring_residents_back(elapsed_seconds);
        self.send_residents_away();
        self.refresh_current_visitors();
    }

    /// Moves residents who have reached the street on an outside trip to `away`. A school day
    /// or a working day keeps them out for hours, and kept among `people` they filled every
    /// slot each morning in a tower with many condos, so no visitor or office worker could
    /// arrive until they came home. The original gives each tenant its own fixed occupants, so
    /// one tenant's absent residents never crowd out another's.
    fn send_residents_away(&mut self) {
        self.away
            .extend(self.people.extract_if(.., |person| person.is_away()));
    }

    /// Counts down the trips of residents who are away and lets those whose trip has ended back
    /// in at the street, longest-gone first, while the tower has room. Their visit is over, so
    /// they head home on the next tick like anyone else.
    fn bring_residents_back(&mut self, elapsed_seconds: f32) {
        for person in &mut self.away {
            person.visit_remaining -= elapsed_seconds;
        }
        let room = MAX_SIMULATED_PEOPLE.saturating_sub(self.people.len());
        let mut due = 0;
        self.people.extend(self.away.extract_if(.., |person| {
            let returning = person.visit_remaining <= 0.0 && due < room;
            due += usize::from(returning);
            returning
        }));
    }

    fn advance_commercial_patronage(&mut self, tower: &Tower) {
        for facility in tower.facilities().iter().filter(|facility| {
            facility.is_occupied() && self.reachable_facilities.contains(&facility.id)
        }) {
            let Some(target) = commercial_patronage_target(facility.kind) else {
                continue;
            };
            let patronage = self
                .commercial_patronage
                .entry(facility.id)
                .or_insert_with(|| commercial_patronage_initial(facility.kind));
            if facility.kind == FacilityKind::Cinema {
                *patronage = patronage.saturating_sub(12).max(target);
                continue;
            }
            let daily_growth = target.div_ceil(8).max(1);
            *patronage = patronage.saturating_add(daily_growth).min(target);
        }
    }

    fn refresh_current_visitors(&mut self) {
        self.current_visitors.clear();
        for person in self.people.iter().filter(|person| {
            person.destination_facility_id != 0
                && !person.role.is_staff()
                && person.activity == PersonActivity::Visiting
                && !person.going_home
        }) {
            *self
                .current_visitors
                .entry(person.destination_facility_id)
                .or_default() += 1;
        }
    }

    fn spawn_person(&mut self, tower: &Tower) -> bool {
        let destinations: Vec<(&Facility, TravelMode)> = tower
            .facilities()
            .iter()
            .filter_map(|facility| {
                if !facility.is_occupied()
                    || !is_person_destination(facility.kind)
                    || !facility_is_open_at(facility.kind, tower.clock)
                {
                    return None;
                }
                self.routes_from_lobby
                    .get(&facility.id)
                    .copied()
                    .map(|route| (facility, route))
            })
            .collect();
        if destinations.is_empty() {
            return false;
        }

        let seed = mix64(self.next_person_id ^ (u64::from(tower.clock.day) << 32));
        // Balance arrivals by each tenant's assigned/capacity ratio. The old
        // weighted lottery could send several people to one office while an
        // equally reachable office stayed empty throughout the workday.
        let mut assigned_by_facility = HashMap::new();
        for person in self.people.iter().filter(|person| !person.going_home) {
            *assigned_by_facility
                .entry(person.destination_facility_id)
                .or_insert(0_usize) += 1;
        }
        let mut least_loaded = Vec::new();
        let mut best_load = None::<(usize, usize)>;
        for &(facility, route) in &destinations {
            let assigned = assigned_by_facility.get(&facility.id).copied().unwrap_or(0);
            let capacity = facility.kind.population_capacity().max(1) as usize;
            if assigned >= capacity {
                continue;
            }
            let ordering = best_load.map(|(best_assigned, best_capacity)| {
                (assigned * best_capacity).cmp(&(best_assigned * capacity))
            });
            match ordering {
                None | Some(std::cmp::Ordering::Less) => {
                    best_load = Some((assigned, capacity));
                    least_loaded.clear();
                    least_loaded.push((facility, route));
                }
                Some(std::cmp::Ordering::Equal) => least_loaded.push((facility, route)),
                Some(std::cmp::Ordering::Greater) => {}
            }
        }
        if least_loaded.is_empty() {
            return false;
        }
        let selected = least_loaded[seed.rotate_left(23) as usize % least_loaded.len()];
        let (destination, lobby_route) = selected;
        // Every simulated person has a complete entrance-to-exit lifecycle.
        // Nobody materializes in an office, residence, hotel room, or business.
        let destination_kind = destination.kind;
        let destination_endpoint = endpoint(destination);
        let (origin, route) =
            self.arrival_origin(tower, destination, destination_endpoint, lobby_route, seed);

        let mut person = Person {
            id: self.next_person_id,
            role: outside_visitor_role(destination_kind, self.next_person_id),
            home_facility_id: origin.facility_id,
            destination_facility_id: destination_endpoint.facility_id,
            x: origin.x,
            floor_position: f32::from(floor_ordinal(origin.floor)),
            current_floor: origin.floor,
            mood: PersonMood::Calm,
            activity: PersonActivity::Walking,
            wait_seconds: 0.0,
            origin,
            destination: destination_endpoint,
            route,
            going_home: false,
            walking_from_transport: route == TravelMode::SameFloor,
            visit_remaining: 0.0,
            visit_duration: visit_duration_seconds(destination_kind),
            stair_progress: 0.0,
            route_leg: 0,
        };
        start_trip(&mut person);
        self.next_person_id += 1;
        self.people.push(person);
        true
    }

    fn arrival_origin(
        &mut self,
        tower: &Tower,
        destination_facility: &Facility,
        destination: Endpoint,
        lobby_route: TravelMode,
        seed: u64,
    ) -> (Endpoint, TravelMode) {
        let parking_required =
            tower.star_rating() >= 3 && tenant_requests_parking(destination_facility);
        let parking_already_reserved = self.people.iter().any(|person| {
            person.destination_facility_id == destination.facility_id
                && self.connected_parking.contains(&person.home_facility_id)
        });
        if parking_required && !parking_already_reserved {
            let reserved = self
                .people
                .iter()
                .filter_map(|person| {
                    self.connected_parking
                        .contains(&person.home_facility_id)
                        .then_some(person.home_facility_id)
                })
                .collect::<HashSet<_>>();
            let available = tower
                .facilities()
                .iter()
                .filter(|facility| {
                    facility.kind == FacilityKind::Parking
                        && self.connected_parking.contains(&facility.id)
                        && !reserved.contains(&facility.id)
                })
                .filter_map(|facility| {
                    let origin = endpoint(facility);
                    choose_route_scored(
                        origin,
                        destination,
                        &self.elevators,
                        tower.facilities(),
                        tower.floors(),
                    )
                    .map(|(route, _)| (origin, route))
                })
                .collect::<Vec<_>>();
            if !available.is_empty() {
                let (origin, route) = available[seed as usize % available.len()];
                return (origin, route);
            }
            if destination_facility.kind == FacilityKind::Office {
                self.parking_demand_warning = true;
            }
        }

        if seed.is_multiple_of(2)
            && let Some((origin, route, _)) = tower
                .facilities()
                .iter()
                .filter(|facility| facility.kind == FacilityKind::Metro)
                .filter_map(|facility| {
                    let origin = facility_top_endpoint(facility);
                    choose_route_scored(
                        origin,
                        destination,
                        &self.elevators,
                        tower.facilities(),
                        tower.floors(),
                    )
                    .map(|(route, score)| (origin, route, score))
                })
                .min_by(|left, right| left.2.total_cmp(&right.2))
        {
            return (origin, route);
        }

        (
            Endpoint {
                facility_id: 0,
                x: exterior_spawn_x(tower, seed),
                floor: 1,
            },
            lobby_route,
        )
    }

    fn dispatch_scheduled_trips(&mut self, tower: &Tower) {
        const DISPATCH_BUCKET_MINUTES: u64 = 5;
        const MAX_DISPATCHES_PER_BUCKET: usize = 12;
        const MAX_HOMES_INSPECTED_PER_BUCKET: usize = 256;

        let bucket = tower.clock.absolute_minute() / DISPATCH_BUCKET_MINUTES;
        if bucket == self.last_resident_dispatch_bucket {
            return;
        }
        self.last_resident_dispatch_bucket = bucket;
        if self.people.len() >= MAX_SIMULATED_PEOPLE || !scheduled_trip_window_active(tower.clock) {
            return;
        }

        let facilities = tower.facilities();
        if facilities.is_empty() {
            return;
        }
        let start = self.scheduled_home_cursor % facilities.len();
        let inspection_limit = facilities.len().min(MAX_HOMES_INSPECTED_PER_BUCKET);
        let mut dispatched = 0;
        for offset in 0..inspection_limit {
            let index = (start + offset) % facilities.len();
            self.scheduled_home_cursor = (index + 1) % facilities.len();
            let home = &facilities[index];
            if !home.is_occupied()
                || !self.reachable_facilities.contains(&home.id)
                || !matches!(
                    home.kind,
                    FacilityKind::Condo
                        | FacilityKind::Office
                        | FacilityKind::HotelSingle
                        | FacilityKind::HotelTwin
                        | FacilityKind::HotelSuite
                        | FacilityKind::Cinema
                )
                || (home.kind != FacilityKind::Condo && self.visitors_at(home.id) == 0)
            {
                continue;
            }
            for trip in scheduled_trip_specs(tower.clock, home, tower.star_rating()) {
                if self.people.len() >= MAX_SIMULATED_PEOPLE
                    || dispatched >= MAX_DISPATCHES_PER_BUCKET
                {
                    return;
                }
                let stamp_key = scheduled_trip_stamp_key(home.id, trip.event);
                if self.scheduled_trip_days.get(&stamp_key) == Some(&tower.clock.day) {
                    continue;
                }
                if self.spawn_scheduled_trip(tower, home, trip) {
                    self.scheduled_trip_days.insert(stamp_key, tower.clock.day);
                    dispatched += 1;
                }
            }
        }
    }

    fn spawn_scheduled_trip(
        &mut self,
        tower: &Tower,
        home: &Facility,
        trip: ScheduledTripSpec,
    ) -> bool {
        let origin = endpoint(home);
        let (destination, route) = match trip.purpose {
            ScheduledTripPurpose::Outside => {
                let Some(route) = self.routes_from_lobby.get(&home.id).copied() else {
                    return false;
                };
                let seed = mix64(self.next_person_id ^ home.id ^ u64::from(tower.clock.day));
                (
                    Endpoint {
                        facility_id: 0,
                        x: exterior_spawn_x(tower, seed),
                        floor: 1,
                    },
                    reverse_route(route),
                )
            }
            purpose => {
                let target = tower
                    .facilities()
                    .iter()
                    .filter(|facility| {
                        facility.is_occupied()
                            && facility.id != home.id
                            && resident_purpose_accepts(purpose, facility.kind)
                            && facility_is_open(facility.kind, tower.clock.minute_of_day)
                    })
                    .filter_map(|facility| {
                        choose_route_scored(
                            origin,
                            endpoint(facility),
                            &self.elevators,
                            tower.facilities(),
                            tower.floors(),
                        )
                        .map(|(route, score)| (facility, route, score))
                    })
                    .min_by(|left, right| left.2.total_cmp(&right.2));
                let Some((target, route, _)) = target else {
                    // The original residents leave the tower when the service
                    // they want is unavailable nearby.
                    let Some(route) = self.routes_from_lobby.get(&home.id).copied() else {
                        return false;
                    };
                    let seed = mix64(self.next_person_id ^ home.id ^ u64::from(tower.clock.day));
                    return self.push_scheduled_person(
                        home,
                        Endpoint {
                            facility_id: 0,
                            x: exterior_spawn_x(tower, seed),
                            floor: 1,
                        },
                        reverse_route(route),
                        trip,
                    );
                };
                (endpoint(target), route)
            }
        };
        self.push_scheduled_person(home, destination, route, trip)
    }

    fn push_scheduled_person(
        &mut self,
        home: &Facility,
        destination: Endpoint,
        route: TravelMode,
        trip: ScheduledTripSpec,
    ) -> bool {
        let origin = endpoint(home);
        let mut person = Person {
            id: self.next_person_id,
            role: trip.role,
            home_facility_id: home.id,
            destination_facility_id: destination.facility_id,
            x: origin.x,
            floor_position: f32::from(floor_ordinal(origin.floor)),
            current_floor: origin.floor,
            mood: PersonMood::Calm,
            activity: PersonActivity::Walking,
            wait_seconds: 0.0,
            origin,
            destination,
            route,
            going_home: false,
            walking_from_transport: route == TravelMode::SameFloor,
            visit_remaining: 0.0,
            visit_duration: trip.visit_seconds,
            stair_progress: 0.0,
            route_leg: 0,
        };
        start_trip(&mut person);
        self.next_person_id += 1;
        self.people.push(person);
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScheduledTripPurpose {
    Outside,
    OfficeCall,
    Shopping,
    Meal,
    Medical,
    Commercial,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ScheduledTripSpec {
    event: ScheduledTripEvent,
    role: PersonRole,
    purpose: ScheduledTripPurpose,
    visit_seconds: f32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScheduledTripEvent {
    ChildSchool,
    ResidentWork,
    ResidentShopping,
    WeekendMotherMeal,
    WeekendChildMeal,
    WeekendAdultMeal,
    OfficeSalesCall,
    OfficeLunch,
    OfficeMedical,
    HotelDinner,
    CinemaMatineeDispersal,
    CinemaEveningDispersal,
}

fn scheduled_trip_specs(clock: Clock, home: &Facility, star_rating: u8) -> Vec<ScheduledTripSpec> {
    let minute = clock.minute_of_day;
    let weekend = is_weekend(clock);
    let home_id = home.id;
    let mut trips = Vec::with_capacity(3);
    if home.kind == FacilityKind::Condo && weekend {
        let departure = 11 * 60 + (home_id % 75) as u16;
        if (departure..13 * 60 + 30).contains(&minute) {
            trips.push(ScheduledTripSpec {
                event: ScheduledTripEvent::WeekendMotherMeal,
                role: PersonRole::WomanWithChild,
                purpose: ScheduledTripPurpose::Meal,
                visit_seconds: 24.0,
            });
            trips.push(ScheduledTripSpec {
                event: ScheduledTripEvent::WeekendChildMeal,
                role: PersonRole::Child,
                purpose: ScheduledTripPurpose::Meal,
                visit_seconds: 24.0,
            });
            trips.push(ScheduledTripSpec {
                event: ScheduledTripEvent::WeekendAdultMeal,
                role: PersonRole::Homebody,
                purpose: ScheduledTripPurpose::Meal,
                visit_seconds: 24.0,
            });
        }
        return trips;
    }

    if home.kind == FacilityKind::Office && !weekend {
        let sales_departure = 10 * 60 + (home_id % 75) as u16;
        if (sales_departure..11 * 60 + 45).contains(&minute) {
            trips.push(ScheduledTripSpec {
                event: ScheduledTripEvent::OfficeSalesCall,
                role: PersonRole::Salesman,
                purpose: ScheduledTripPurpose::OfficeCall,
                visit_seconds: 18.0,
            });
        }
        let lunch_departure = 12 * 60 + (home_id % 45) as u16;
        if (lunch_departure..13 * 60).contains(&minute) {
            trips.push(ScheduledTripSpec {
                event: ScheduledTripEvent::OfficeLunch,
                role: PersonRole::Salesman,
                purpose: ScheduledTripPurpose::Meal,
                visit_seconds: 18.0,
            });
        }
        let medical_departure = 14 * 60 + (home_id % 90) as u16;
        if star_rating >= 3
            && (home_id + u64::from(clock.day)).is_multiple_of(3)
            && (medical_departure..16 * 60).contains(&minute)
        {
            trips.push(ScheduledTripSpec {
                event: ScheduledTripEvent::OfficeMedical,
                role: PersonRole::Salesman,
                purpose: ScheduledTripPurpose::Medical,
                visit_seconds: 24.0,
            });
        }
        return trips;
    }

    if matches!(
        home.kind,
        FacilityKind::HotelSingle | FacilityKind::HotelTwin | FacilityKind::HotelSuite
    ) {
        let departure = 18 * 60 + (home_id % 90) as u16;
        if (departure..20 * 60 + 30).contains(&minute) {
            trips.push(ScheduledTripSpec {
                event: ScheduledTripEvent::HotelDinner,
                role: PersonRole::Visitor,
                purpose: ScheduledTripPurpose::Meal,
                visit_seconds: 35.0,
            });
        }
        return trips;
    }

    if home.kind == FacilityKind::Cinema {
        let event = if (15 * 60 + 50..16 * 60 + 15).contains(&minute) {
            Some(ScheduledTripEvent::CinemaMatineeDispersal)
        } else if (19 * 60 + 50..20 * 60 + 15).contains(&minute) {
            Some(ScheduledTripEvent::CinemaEveningDispersal)
        } else {
            None
        };
        if let Some(event) = event {
            trips.push(ScheduledTripSpec {
                event,
                role: PersonRole::Visitor,
                purpose: ScheduledTripPurpose::Commercial,
                visit_seconds: 22.0,
            });
        }
        return trips;
    }

    if home.kind != FacilityKind::Condo {
        return trips;
    }

    let child_departure = 7 * 60 + 20 + (home_id % 40) as u16;
    if (child_departure..9 * 60).contains(&minute) {
        trips.push(ScheduledTripSpec {
            event: ScheduledTripEvent::ChildSchool,
            role: PersonRole::Child,
            purpose: ScheduledTripPurpose::Outside,
            // Six game hours at normal speed: schoolchildren return in the
            // early afternoon, before the working adult.
            visit_seconds: 180.0,
        });
    }
    let worker_departure = 8 * 60 + (home_id % 45) as u16;
    if (worker_departure..9 * 60 + 45).contains(&minute) {
        trips.push(ScheduledTripSpec {
            event: ScheduledTripEvent::ResidentWork,
            role: PersonRole::Salesman,
            purpose: ScheduledTripPurpose::Outside,
            visit_seconds: 270.0,
        });
    }
    let shopping_departure = 10 * 60 + (home_id % 60) as u16;
    if (shopping_departure..12 * 60).contains(&minute) {
        trips.push(ScheduledTripSpec {
            event: ScheduledTripEvent::ResidentShopping,
            role: PersonRole::Woman,
            purpose: ScheduledTripPurpose::Shopping,
            visit_seconds: 28.0,
        });
    }
    trips
}

fn scheduled_trip_window_active(clock: Clock) -> bool {
    if is_weekend(clock) {
        (11 * 60..20 * 60 + 15).contains(&clock.minute_of_day)
    } else {
        (7 * 60 + 20..20 * 60 + 15).contains(&clock.minute_of_day)
    }
}

fn resident_purpose_accepts(purpose: ScheduledTripPurpose, kind: FacilityKind) -> bool {
    match purpose {
        ScheduledTripPurpose::Outside => false,
        ScheduledTripPurpose::OfficeCall => kind == FacilityKind::Office,
        ScheduledTripPurpose::Shopping => kind == FacilityKind::Shop,
        ScheduledTripPurpose::Meal => {
            matches!(kind, FacilityKind::FastFood | FacilityKind::Restaurant)
        }
        ScheduledTripPurpose::Medical => kind == FacilityKind::Medical,
        ScheduledTripPurpose::Commercial => matches!(
            kind,
            FacilityKind::FastFood | FacilityKind::Restaurant | FacilityKind::Shop
        ),
    }
}

const fn scheduled_trip_stamp_key(home_id: u64, event: ScheduledTripEvent) -> u64 {
    (home_id << 5) | event as u64
}

const fn outside_visitor_role(kind: FacilityKind, person_id: u64) -> PersonRole {
    match kind {
        FacilityKind::Office => PersonRole::Salesman,
        FacilityKind::Condo => match person_id % 3 {
            0 => PersonRole::Homebody,
            1 => PersonRole::Woman,
            _ => PersonRole::Child,
        },
        _ => PersonRole::Visitor,
    }
}

fn advance_people(people: &mut [Person], elapsed_seconds: f32) {
    for person in people {
        match person.activity {
            PersonActivity::Walking => {
                let target = if person.walking_from_transport {
                    trip_destination(person).x
                } else {
                    match person.route {
                        TravelMode::SameFloor => trip_destination(person).x,
                        TravelMode::Elevator { x, .. } | TravelMode::Stairs { x } => x,
                        TravelMode::ElevatorTransfer {
                            first_x, second_x, ..
                        } => {
                            if person.route_leg == 0 {
                                first_x
                            } else {
                                second_x
                            }
                        }
                        TravelMode::ElevatorThenWalkway {
                            shaft_x, walkway_x, ..
                        } => {
                            if person.route_leg == 0 {
                                shaft_x
                            } else {
                                walkway_x
                            }
                        }
                        TravelMode::WalkwayThenElevator {
                            walkway_x, shaft_x, ..
                        } => {
                            if person.route_leg == 0 {
                                walkway_x
                            } else {
                                shaft_x
                            }
                        }
                    }
                };
                person.x = move_toward(person.x, target, elapsed_seconds * 8.0);
                if (person.x - target).abs() < 0.01 {
                    if person.walking_from_transport {
                        arrive_at_facility(person);
                    } else {
                        match person.route {
                            TravelMode::SameFloor => arrive_at_facility(person),
                            TravelMode::Elevator { shaft_id, .. } => {
                                person.activity = PersonActivity::WaitingForElevator { shaft_id };
                            }
                            TravelMode::ElevatorTransfer {
                                first_id,
                                second_id,
                                ..
                            } => {
                                person.activity = PersonActivity::WaitingForElevator {
                                    shaft_id: if person.route_leg == 0 {
                                        first_id
                                    } else {
                                        second_id
                                    },
                                };
                            }
                            TravelMode::ElevatorThenWalkway { shaft_id, .. }
                                if person.route_leg == 0 =>
                            {
                                person.activity = PersonActivity::WaitingForElevator { shaft_id };
                            }
                            TravelMode::WalkwayThenElevator { shaft_id, .. }
                                if person.route_leg == 1 =>
                            {
                                person.activity = PersonActivity::WaitingForElevator { shaft_id };
                            }
                            TravelMode::Stairs { .. }
                            | TravelMode::ElevatorThenWalkway { .. }
                            | TravelMode::WalkwayThenElevator { .. } => {
                                person.activity = PersonActivity::UsingStairs;
                                person.stair_progress = 0.0;
                            }
                        }
                    }
                }
            }
            PersonActivity::WaitingForElevator { .. } => {
                person.wait_seconds += elapsed_seconds;
                person.mood = if person.wait_seconds >= ANGRY_WAIT_SECONDS {
                    PersonMood::Angry
                } else if person.wait_seconds >= CONCERNED_WAIT_SECONDS {
                    PersonMood::Concerned
                } else {
                    PersonMood::Calm
                };
            }
            PersonActivity::RidingElevator { .. } => {}
            PersonActivity::UsingStairs => {
                person.stair_progress = (person.stair_progress + elapsed_seconds * 0.7).min(1.0);
                let leg_end_floor = walkway_leg_end_floor(person);
                let start = f32::from(floor_ordinal(person.current_floor));
                let end = f32::from(floor_ordinal(leg_end_floor));
                person.floor_position = start + (end - start) * person.stair_progress;
                if person.stair_progress >= 1.0 {
                    person.current_floor = leg_end_floor;
                    person.floor_position = end;
                    finish_leg(person);
                    person.activity = PersonActivity::Walking;
                }
            }
            PersonActivity::Visiting => {
                person.visit_remaining -= elapsed_seconds;
                if person.visit_remaining <= 0.0 && !person.going_home {
                    person.going_home = true;
                    person.current_floor = person.destination.floor;
                    person.floor_position = f32::from(floor_ordinal(person.current_floor));
                    person.route = reverse_route(person.route);
                    person.route_leg = 0;
                    person.walking_from_transport = person.route == TravelMode::SameFloor;
                    person.activity = PersonActivity::Walking;
                }
            }
            PersonActivity::LeavingElevatorQueue => {
                person.visit_remaining -= elapsed_seconds;
                person.x = move_toward(person.x, person.origin.x, elapsed_seconds * 8.0);
            }
        }
    }
}

fn enforce_elevator_queue_limits(people: &mut [Person]) {
    let mut queue_lengths: HashMap<(u64, i16), usize> = HashMap::new();
    for person in people {
        let PersonActivity::WaitingForElevator { shaft_id } = person.activity else {
            continue;
        };
        let queue_length = queue_lengths
            .entry((shaft_id, person.current_floor))
            .or_default();
        if person.wait_seconds >= ABANDON_ELEVATOR_WAIT_SECONDS
            || *queue_length >= MAX_ELEVATOR_QUEUE_PER_FLOOR
        {
            abandon_trip(person);
            continue;
        }
        *queue_length += 1;
    }
}

fn abandon_trip(person: &mut Person) {
    person.activity = PersonActivity::LeavingElevatorQueue;
    person.mood = PersonMood::Angry;
    person.visit_remaining = QUEUE_ABANDON_WALK_SECONDS;
}

/// True when `shaft` still spans both ends of an elevator leg.
fn shaft_spans(shaft: &ElevatorShaft, from: i16, to: i16) -> bool {
    shaft.served_floors.contains(&from) && shaft.served_floors.contains(&to)
}

/// Gives up waits the shaft can no longer complete: the shaft is gone, or it no longer spans
/// the waiting floor or the floor this leg goes to. Without this, a waiter left on a floor
/// removed from a shortened shaft keeps calling cars into the empty part of the shaft.
fn abandon_unservable_elevator_waits(people: &mut [Person], elevators: &[ElevatorShaft]) {
    for person in people {
        let PersonActivity::WaitingForElevator { shaft_id } = person.activity else {
            continue;
        };
        let servable = elevators
            .iter()
            .find(|shaft| shaft.id == shaft_id)
            .is_some_and(|shaft| {
                shaft_spans(shaft, person.current_floor, desired_elevator_floor(person))
            });
        if !servable {
            abandon_trip(person);
        }
    }
}

/// Brings travellers and cars back in line with freshly rebuilt shafts. A shaft takes the id
/// of its lowest segment, so demolishing that segment renumbers it; demolishing others can
/// shorten or remove it. The original finalizes the route of every traveller using a removed
/// span, so a trip the shafts can no longer complete is abandoned rather than left riding, or
/// calling cars, beyond the shaft. People never refer to a shaft that no longer exists, which
/// save validation rejects.
fn settle_people_after_shaft_changes(
    elevators: &mut [ElevatorShaft],
    people: &mut [Person],
    renumbered: &HashMap<u64, u64>,
) {
    if !renumbered.is_empty() {
        for person in people.iter_mut() {
            renumber_person_shafts(person, renumbered);
        }
    }
    for shaft in elevators.iter_mut() {
        settle_cars_in_shaft(shaft, people);
    }
    for person in people.iter_mut() {
        if let PersonActivity::RidingElevator { shaft_id } = person.activity
            && !elevators.iter().any(|shaft| shaft.id == shaft_id)
        {
            let floor = floor_from_ordinal(person.floor_position.round() as i16);
            step_off_and_abandon(person, floor, person.x);
        }
    }
    abandon_unservable_elevator_waits(people, elevators);
}

fn renumber_person_shafts(person: &mut Person, renumbered: &HashMap<u64, u64>) {
    let renumber = |id: &mut u64| {
        if let Some(&new_id) = renumbered.get(id) {
            *id = new_id;
        }
    };
    match &mut person.activity {
        PersonActivity::WaitingForElevator { shaft_id }
        | PersonActivity::RidingElevator { shaft_id } => renumber(shaft_id),
        _ => {}
    }
    match &mut person.route {
        TravelMode::Elevator { shaft_id, .. }
        | TravelMode::ElevatorThenWalkway { shaft_id, .. }
        | TravelMode::WalkwayThenElevator { shaft_id, .. } => renumber(shaft_id),
        TravelMode::ElevatorTransfer {
            first_id,
            second_id,
            ..
        } => {
            renumber(first_id);
            renumber(second_id);
        }
        TravelMode::SameFloor | TravelMode::Stairs { .. } => {}
    }
}

/// Pulls cars, with their riders, back inside the shaft's span and drops off, at the landing
/// nearest the car, riders whose floor the shaft no longer reaches.
fn settle_cars_in_shaft(shaft: &mut ElevatorShaft, people: &mut [Person]) {
    let (Some(&bottom), Some(&top)) = (shaft.served_floors.first(), shaft.served_floors.last())
    else {
        return;
    };
    let lowest = f32::from(floor_ordinal(bottom));
    let highest = f32::from(floor_ordinal(top));
    let shaft_id = shaft.id;
    let shaft_x = f32::from(shaft.x) + 2.0;
    let served_floors = &shaft.served_floors;
    for car in &mut shaft.cars {
        car.floor_position = car.floor_position.clamp(lowest, highest);
        if car
            .target_floor
            .is_some_and(|floor| !served_floors.contains(&floor))
        {
            car.target_floor = None;
        }
        let distance = |floor: &i16| (f32::from(floor_ordinal(*floor)) - car.floor_position).abs();
        let Some(nearest) = served_floors
            .iter()
            .copied()
            .min_by(|left, right| distance(left).total_cmp(&distance(right)))
        else {
            continue;
        };
        car.passengers.retain(|&id| {
            let Some(person) = people.iter_mut().find(|person| {
                person.id == id
                    && matches!(
                        person.activity,
                        PersonActivity::RidingElevator { shaft_id: riding } if riding == shaft_id
                    )
            }) else {
                return true;
            };
            if served_floors.contains(&desired_elevator_floor(person)) {
                person.floor_position = car.floor_position;
                return true;
            }
            step_off_and_abandon(person, nearest, shaft_x);
            false
        });
    }
}

fn step_off_and_abandon(person: &mut Person, floor: i16, x: f32) {
    person.current_floor = floor;
    person.floor_position = f32::from(floor_ordinal(floor));
    person.x = x;
    abandon_trip(person);
}

fn advance_shaft(
    shaft: &mut ElevatorShaft,
    people: &mut [Person],
    clock: Clock,
    elapsed_seconds: f32,
) {
    let schedule = shaft.current_schedule(clock);
    let waiting_assignments = assign_waiting_calls(shaft, people, schedule.waiting_car_response);
    for (car_index, car) in shaft.cars.iter_mut().enumerate() {
        if car.dwell_remaining > 0.0 {
            car.dwell_remaining -= elapsed_seconds;
            board_waiting_people(shaft.id, shaft.kind, shaft.x, car, people);
            if car.dwell_remaining > 0.0 {
                continue;
            }
        }

        let stops = requested_stops(shaft.id, people, car, car_index, &waiting_assignments);
        if stops.is_empty() {
            if (car.floor_position - f32::from(floor_ordinal(car.home_floor))).abs() < 0.01 {
                car.direction = ElevatorDirection::Idle;
                car.target_floor = None;
                continue;
            }
            car.target_floor = Some(car.home_floor);
        } else if !car.passengers.is_empty() && schedule.mode == ElevatorMode::ExpressToTop {
            // Like the original's express-up schedule, run without stopping to the highest
            // requested floor, then the next. Always heading for the top served floor never let
            // off anyone bound elsewhere.
            car.target_floor = stops
                .iter()
                .copied()
                .max_by_key(|floor| floor_ordinal(*floor));
        } else if !car.passengers.is_empty() && schedule.mode == ElevatorMode::ExpressToBottom {
            car.target_floor = stops
                .iter()
                .copied()
                .min_by_key(|floor| floor_ordinal(*floor));
        } else {
            car.target_floor = choose_next_stop(car.floor_position, car.direction, &stops);
        }

        let Some(target_floor) = car.target_floor else {
            continue;
        };
        let target = f32::from(floor_ordinal(target_floor));
        car.direction = if target > car.floor_position {
            ElevatorDirection::Up
        } else if target < car.floor_position {
            ElevatorDirection::Down
        } else {
            car.direction
        };
        car.floor_position = move_toward(car.floor_position, target, elapsed_seconds * 2.2);

        for person in people.iter_mut().filter(|person| {
            matches!(
                person.activity,
                PersonActivity::RidingElevator { shaft_id } if shaft_id == shaft.id
            ) && car.passengers.contains(&person.id)
        }) {
            person.floor_position = car.floor_position;
            person.x = f32::from(shaft.x) + f32::from(shaft.kind.spec().width) * 0.5;
        }

        if (car.floor_position - target).abs() < 0.01 {
            car.floor_position = target;
            car.target_floor = None;
            unload_people(shaft.id, shaft.x, target_floor, car, people);
            board_waiting_people(shaft.id, shaft.kind, shaft.x, car, people);
            car.dwell_remaining = f32::from(schedule.departure_seconds).max(0.6);
        }
    }
}

fn board_waiting_people(
    shaft_id: u64,
    kind: FacilityKind,
    shaft_x: u16,
    car: &mut ElevatorCar,
    people: &mut [Person],
) {
    let floor = floor_from_ordinal(car.floor_position.round() as i16);
    for person in people.iter_mut().filter(|person| {
        person.current_floor == floor
            && matches!(
                person.activity,
                PersonActivity::WaitingForElevator { shaft_id: waiting_for } if waiting_for == shaft_id
            )
    }) {
        if car.passengers.len() >= elevator_capacity(kind) {
            break;
        }
        person.activity = PersonActivity::RidingElevator { shaft_id };
        person.x = f32::from(shaft_x) + 2.0;
        car.passengers.push(person.id);
    }
}

pub const fn elevator_capacity(kind: FacilityKind) -> usize {
    if matches!(kind, FacilityKind::ExpressElevator) {
        EXPRESS_ELEVATOR_CAPACITY
    } else {
        ELEVATOR_CAPACITY
    }
}

fn unload_people(
    shaft_id: u64,
    shaft_x: u16,
    floor: i16,
    car: &mut ElevatorCar,
    people: &mut [Person],
) {
    let mut departing = Vec::new();
    for person in people.iter_mut().filter(|person| {
        matches!(
            person.activity,
            PersonActivity::RidingElevator { shaft_id: riding } if riding == shaft_id
        ) && desired_elevator_floor(person) == floor
            && car.passengers.contains(&person.id)
    }) {
        departing.push(person.id);
        person.current_floor = floor;
        person.floor_position = f32::from(floor_ordinal(floor));
        person.x = f32::from(shaft_x) + 2.0;
        finish_leg(person);
        person.activity = PersonActivity::Walking;
    }
    car.passengers.retain(|id| !departing.contains(id));
}

fn requested_stops(
    shaft_id: u64,
    people: &[Person],
    car: &ElevatorCar,
    car_index: usize,
    waiting_assignments: &[(i16, usize)],
) -> Vec<i16> {
    let mut stops = Vec::new();
    for person in people {
        let floor = match person.activity {
            PersonActivity::WaitingForElevator {
                shaft_id: waiting_for,
            } if waiting_for == shaft_id
                && waiting_assignments.contains(&(person.current_floor, car_index)) =>
            {
                person.current_floor
            }
            PersonActivity::RidingElevator { shaft_id: riding }
                if riding == shaft_id && car.passengers.contains(&person.id) =>
            {
                desired_elevator_floor(person)
            }
            _ => continue,
        };
        if !stops.contains(&floor) {
            stops.push(floor);
        }
    }
    stops
}

fn assign_waiting_calls(
    shaft: &ElevatorShaft,
    people: &[Person],
    response_floors: u8,
) -> Vec<(i16, usize)> {
    let mut waiting_floors = Vec::new();
    for person in people {
        if matches!(
            person.activity,
            PersonActivity::WaitingForElevator { shaft_id } if shaft_id == shaft.id
        ) && !waiting_floors.contains(&person.current_floor)
        {
            waiting_floors.push(person.current_floor);
        }
    }

    // A full car cannot board anyone, so it must never own a hall call. Otherwise a full car
    // stopped on the caller's floor keeps re-selecting that floor as its next stop and never
    // departs, and because a matching moving car outranks idle cars, no other car is sent.
    let capacity = elevator_capacity(shaft.kind);
    let has_room = |car: &ElevatorCar| car.passengers.len() < capacity;

    waiting_floors
        .into_iter()
        .filter_map(|floor| {
            let floor_position = f32::from(floor_ordinal(floor));
            let desired_direction = people
                .iter()
                .find(|person| {
                    person.current_floor == floor
                        && matches!(
                            person.activity,
                            PersonActivity::WaitingForElevator { shaft_id }
                                if shaft_id == shaft.id
                        )
                })
                .map(|person| {
                    if floor_ordinal(desired_elevator_floor(person)) > floor_ordinal(floor) {
                        ElevatorDirection::Up
                    } else {
                        ElevatorDirection::Down
                    }
                })?;

            // Keep a car committed to a hall call once it starts travelling
            // toward that floor. Without this, the assignment can jump to a
            // different car every frame as their relative distances change.
            // A high-floor car then moves down a fraction of a floor, loses
            // the call, and immediately returns to its default waiting floor.
            let committed = shaft
                .cars
                .iter()
                .enumerate()
                .filter(|(_, car)| car.target_floor == Some(floor) && has_room(car))
                .min_by(|left, right| {
                    (left.1.floor_position - floor_position)
                        .abs()
                        .total_cmp(&(right.1.floor_position - floor_position).abs())
                })
                .map(|(index, _)| index);

            let moving = shaft
                .cars
                .iter()
                .enumerate()
                .filter(|(_, car)| {
                    has_room(car)
                        && car.direction == desired_direction
                        && match desired_direction {
                            ElevatorDirection::Up => car.floor_position <= floor_position,
                            ElevatorDirection::Down => car.floor_position >= floor_position,
                            ElevatorDirection::Idle => false,
                        }
                        && (car.floor_position - floor_position).abs() <= f32::from(response_floors)
                })
                .min_by(|left, right| {
                    (left.1.floor_position - floor_position)
                        .abs()
                        .total_cmp(&(right.1.floor_position - floor_position).abs())
                })
                .map(|(index, _)| index);
            let waiting = shaft
                .cars
                .iter()
                .enumerate()
                .filter(|(_, car)| car.direction == ElevatorDirection::Idle && has_room(car))
                .min_by(|left, right| {
                    (left.1.floor_position - floor_position)
                        .abs()
                        .total_cmp(&(right.1.floor_position - floor_position).abs())
                })
                .map(|(index, _)| index);
            let nearest = shaft
                .cars
                .iter()
                .enumerate()
                .filter(|(_, car)| has_room(car))
                .min_by(|left, right| {
                    (left.1.floor_position - floor_position)
                        .abs()
                        .total_cmp(&(right.1.floor_position - floor_position).abs())
                })
                .map(|(index, _)| index);
            // The original takes a car standing at the caller's floor at once, and
            // otherwise prefers the nearest idle car unless a moving one is closer by less than
            // the dispatch threshold (tower-together specs/ELEVATORS.md, "Floor Assignment").
            // A car heading back to its home floor targets that floor too, so being committed
            // alone must not outrank an idle car much nearer the caller.
            let distance = |index: usize| (shaft.cars[index].floor_position - floor_position).abs();
            let at_floor = shaft.cars.iter().position(|car| {
                has_room(car)
                    && (car.floor_position - floor_position).abs() < 0.01
                    && car.target_floor.is_none_or(|target| target == floor)
                    && (car.direction == ElevatorDirection::Idle
                        || car.direction == desired_direction)
            });
            let idle_distance = waiting.map(distance);
            let within_threshold_of_idle = |index: &usize| {
                idle_distance
                    .is_none_or(|idle| distance(*index) - idle < f32::from(response_floors))
            };
            at_floor
                .or(committed.filter(within_threshold_of_idle))
                .or(moving.filter(within_threshold_of_idle))
                .or(waiting)
                .or(nearest)
                .map(|index| (floor, index))
        })
        .collect()
}

fn choose_next_stop(position: f32, direction: ElevatorDirection, stops: &[i16]) -> Option<i16> {
    let mut ranked: Vec<(i16, f32)> = stops
        .iter()
        .copied()
        .map(|floor| (floor, f32::from(floor_ordinal(floor))))
        .collect();
    ranked.sort_by(|left, right| left.1.total_cmp(&right.1));
    match direction {
        ElevatorDirection::Up => ranked
            .iter()
            .find(|(_, ordinal)| *ordinal >= position - 0.01)
            .or_else(|| ranked.last())
            .map(|(floor, _)| *floor),
        ElevatorDirection::Down => ranked
            .iter()
            .rev()
            .find(|(_, ordinal)| *ordinal <= position + 0.01)
            .or_else(|| ranked.first())
            .map(|(floor, _)| *floor),
        ElevatorDirection::Idle => ranked
            .iter()
            .min_by(|left, right| {
                (left.1 - position)
                    .abs()
                    .total_cmp(&(right.1 - position).abs())
            })
            .map(|(floor, _)| *floor),
    }
}

#[cfg(test)]
fn choose_route(
    origin: Endpoint,
    destination: Endpoint,
    elevators: &[ElevatorShaft],
    facilities: &[Facility],
    floors: &[GridPosition],
) -> Option<TravelMode> {
    choose_route_scored(origin, destination, elevators, facilities, floors).map(|(route, _)| route)
}

fn route_from_first_floor_lobby(
    destination: Endpoint,
    elevators: &[ElevatorShaft],
    facilities: &[Facility],
    floors: &[GridPosition],
) -> Option<TravelMode> {
    let lobbies = facilities
        .iter()
        .filter(|facility| facility.kind == FacilityKind::Lobby && facility.position.floor == 1)
        .collect::<Vec<_>>();
    let candidates = lobby_origin_candidates(&lobbies, destination, elevators, facilities);
    // Every lobby slice is an alternative start, so the elevator-and-walkway fallback applies
    // only when no slice has a simpler route; otherwise a slice that only has the fallback could
    // outscore a direct route from another slice.
    let best_from_any_slice = |search: &dyn Fn(Endpoint) -> Option<(TravelMode, f32)>| {
        lobbies
            .iter()
            .zip(&candidates)
            .filter(|(_, candidate)| **candidate)
            .filter_map(|(lobby, _)| search(endpoint(lobby)))
            .min_by(|left, right| left.1.total_cmp(&right.1))
    };
    best_from_any_slice(&|origin| {
        choose_route_scored_for(
            origin,
            destination,
            elevators,
            facilities,
            floors,
            ElevatorAccess::Public,
        )
    })
    .or_else(|| {
        best_from_any_slice(&|origin| {
            choose_walkway_transfer(origin, destination, elevators, facilities, floors)
        })
    })
    .map(|(route, _)| route)
}

/// Marks the first-floor lobby slices that can start the best route to `destination`.
///
/// A route's score depends on its starting slice only through `|slice - X|`, where X is the
/// route's first horizontal target (the destination on the same floor, a stair or escalator
/// column, or a shaft centre). Feasibility depends on the slice only through its contiguous
/// lobby component and the floor between slice and X. Within a component, the slices nearest
/// X therefore score best, and they are feasible whenever any slice of that component is.
/// Searching from just those slices (ties kept, original order kept) returns the same route
/// as searching from every slice, including tie-breaks. A wide lobby is one facility per
/// column, so this replaces one route search per column with a handful per destination.
fn lobby_origin_candidates(
    lobbies: &[&Facility],
    destination: Endpoint,
    elevators: &[ElevatorShaft],
    facilities: &[Facility],
) -> Vec<bool> {
    let mut anchors = vec![destination.x];
    anchors.extend(
        connected_vertical_walkways(1, destination.floor, facilities)
            .into_iter()
            .map(|(_, x)| x),
    );
    anchors.extend(elevators.iter().map(shaft_center));

    let components = lobby_components(1, facilities);
    let component_of = |lobby: &Facility| {
        components
            .iter()
            .position(|&(start, end)| start <= lobby.position.x && lobby.position.x < end)
    };
    let membership = lobbies
        .iter()
        .map(|lobby| component_of(lobby))
        .collect::<Vec<_>>();
    // A slice outside every component cannot occur, but searching it keeps the result exact.
    let mut candidates = membership.iter().map(Option::is_none).collect::<Vec<_>>();
    for component in 0..components.len() {
        for &anchor in &anchors {
            let distance = |index: usize| (endpoint(lobbies[index]).x - anchor).abs();
            let members = (0..lobbies.len()).filter(|&index| membership[index] == Some(component));
            let Some(nearest) = members.clone().map(distance).min_by(f32::total_cmp) else {
                continue;
            };
            for index in members.filter(|&index| distance(index) == nearest) {
                candidates[index] = true;
            }
        }
    }
    candidates
}

fn route_uses_removed_walkway(
    route: TravelMode,
    destination_floor: i16,
    removed_kind: FacilityKind,
    removed_x: u16,
    removed_floor: i16,
) -> bool {
    // The floors a lobby route climbs on stairs or an escalator, and where.
    let (route_x, leg_start, leg_end) = match route {
        TravelMode::Stairs { x } => (x, 1, destination_floor),
        TravelMode::ElevatorThenWalkway {
            walkway_x,
            transfer_floor,
            ..
        } => (walkway_x, transfer_floor, destination_floor),
        TravelMode::WalkwayThenElevator {
            walkway_x,
            transfer_floor,
            ..
        } => (walkway_x, 1, transfer_floor),
        TravelMode::SameFloor
        | TravelMode::Elevator { .. }
        | TravelMode::ElevatorTransfer { .. } => {
            return false;
        }
    };
    let walkway_x = f32::from(removed_x) + f32::from(removed_kind.spec().width) * 0.5;
    if (route_x - walkway_x).abs() > f32::EPSILON {
        return false;
    }

    // A flight based on N joins ordinal N to N + 1. First floor is ordinal
    // zero, while B1 is -1, so this also handles the B1/first-floor join.
    let start = floor_ordinal(leg_start);
    let end = floor_ordinal(leg_end);
    let flight_base = floor_ordinal(removed_floor);
    start.min(end) <= flight_base && start.max(end) > flight_base
}

/// Where the walk to the destination begins: the x of the last shaft, stairs or escalator the
/// route uses, or `None` for a route that stays on one floor.
fn final_transport_x(route: TravelMode) -> Option<f32> {
    match route {
        TravelMode::SameFloor => None,
        TravelMode::Elevator { x, .. } | TravelMode::Stairs { x } => Some(x),
        TravelMode::ElevatorTransfer { second_x, .. } => Some(second_x),
        TravelMode::ElevatorThenWalkway { walkway_x, .. } => Some(walkway_x),
        TravelMode::WalkwayThenElevator { shaft_x, .. } => Some(shaft_x),
    }
}

fn route_uses_removed_floor(
    route: TravelMode,
    destination: &Facility,
    removed: GridPosition,
) -> bool {
    // First-floor floor cells can participate in the entrance leg before any
    // vertical transport. The cached route does not retain which lobby slice
    // supplied that leg, so conservatively refresh all routes on floor one.
    if removed.floor == 1 {
        return true;
    }
    if destination.position.floor != removed.floor {
        return false;
    }

    // Transfer walks cross only lobby and walkway cells, whose floor cannot be removed, so only
    // the final walk can be cut.
    let Some(transport_x) = final_transport_x(route) else {
        return true;
    };
    let destination_x = endpoint(destination).x;
    let start = transport_x.min(destination_x).floor().max(0.0) as u16;
    let end = transport_x.max(destination_x).floor().max(0.0) as u16;
    (start..=end).contains(&removed.x)
}

fn tower_topology_signature(tower: &Tower) -> u64 {
    let mut signature = mix64(u64::from(tower.width()) ^ 0x544f_504f_4c4f_4759);
    for floor in tower.floors() {
        let floor_bits = u64::from(floor.x) | ((floor.floor as i64 as u64) << 16);
        signature = mix64(signature ^ floor_bits);
    }
    signature = mix64(signature ^ tower.floors().len() as u64);
    let route_infrastructure = tower
        .facilities()
        .iter()
        .filter(|facility| is_route_infrastructure(facility.kind));
    let mut infrastructure_count = 0_u64;
    for facility in route_infrastructure {
        infrastructure_count += 1;
        let position_bits = u64::from(facility.position.x)
            | ((facility.position.floor as i64 as u64) << 16)
            | (u64::from(facility.stories) << 32);
        signature = mix64(signature ^ facility.id ^ ((facility.kind as u64) << 48) ^ position_bits);
    }
    mix64(signature ^ infrastructure_count)
}

/// Returns parking spaces reachable from a valid ramp stack. The original
/// coverage pass may bridge at most three empty floor-tool units and stops at
/// the first non-parking object in either direction.
pub fn connected_parking_spaces(facilities: &[Facility]) -> HashSet<u64> {
    let mut valid_ramps = HashSet::<(u16, i16)>::new();
    let mut ramps = facilities
        .iter()
        .filter(|facility| facility.kind == FacilityKind::Ramp && facility.position.floor < 0)
        .collect::<Vec<_>>();
    ramps.sort_by_key(|ramp| std::cmp::Reverse(ramp.position.floor));
    for ramp in &ramps {
        let valid = if ramp.position.floor == -1 {
            facilities.iter().any(|facility| {
                facility.kind == FacilityKind::Lobby
                    && facility.position.floor == 1
                    && horizontal_ranges_touch(
                        ramp.position.x,
                        ramp.end_x(),
                        facility.position.x,
                        facility.end_x(),
                    )
            })
        } else {
            valid_ramps.contains(&(ramp.position.x, ramp.position.floor + 1))
        };
        if valid {
            valid_ramps.insert((ramp.position.x, ramp.position.floor));
        }
    }

    let mut connected = HashSet::new();
    for ramp in ramps
        .into_iter()
        .filter(|ramp| valid_ramps.contains(&(ramp.position.x, ramp.position.floor)))
    {
        let mut right = facilities
            .iter()
            .filter(|facility| {
                facility.position.floor == ramp.position.floor
                    && facility.position.x >= ramp.end_x()
            })
            .collect::<Vec<_>>();
        right.sort_by_key(|facility| facility.position.x);
        let mut edge = ramp.end_x();
        for facility in right {
            if facility.position.x.saturating_sub(edge) > 3
                || facility.kind != FacilityKind::Parking
            {
                break;
            }
            connected.insert(facility.id);
            edge = facility.end_x();
        }

        let mut left = facilities
            .iter()
            .filter(|facility| {
                facility.position.floor == ramp.position.floor
                    && facility.end_x() <= ramp.position.x
            })
            .collect::<Vec<_>>();
        left.sort_by_key(|facility| std::cmp::Reverse(facility.end_x()));
        let mut edge = ramp.position.x;
        for facility in left {
            if edge.saturating_sub(facility.end_x()) > 3 || facility.kind != FacilityKind::Parking {
                break;
            }
            connected.insert(facility.id);
            edge = facility.position.x;
        }
    }
    connected
}

fn tenant_requests_parking(facility: &Facility) -> bool {
    match facility.kind {
        FacilityKind::HotelSuite => true,
        FacilityKind::Office => {
            // The original office gate checks (logical floor + office column)
            // modulo four. Its logical ground-floor index is ten.
            let logical_floor = i32::from(floor_ordinal(facility.position.floor)) + 10;
            (logical_floor + i32::from(facility.position.x)).rem_euclid(4) == 1
        }
        _ => false,
    }
}

const fn horizontal_ranges_touch(
    first_start: u16,
    first_end: u16,
    second_start: u16,
    second_end: u16,
) -> bool {
    first_start <= second_end && second_start <= first_end
}

const fn is_route_infrastructure(kind: FacilityKind) -> bool {
    matches!(
        kind,
        FacilityKind::Lobby
            | FacilityKind::Stairs
            | FacilityKind::Escalator
            | FacilityKind::Elevator
            | FacilityKind::ServiceElevator
            | FacilityKind::ExpressElevator
            | FacilityKind::Parking
            | FacilityKind::Ramp
            | FacilityKind::Metro
    )
}

fn choose_route_scored(
    origin: Endpoint,
    destination: Endpoint,
    elevators: &[ElevatorShaft],
    facilities: &[Facility],
    floors: &[GridPosition],
) -> Option<(TravelMode, f32)> {
    choose_route_scored_for(
        origin,
        destination,
        elevators,
        facilities,
        floors,
        ElevatorAccess::Public,
    )
    .or_else(|| choose_walkway_transfer(origin, destination, elevators, facilities, floors))
}

/// The best route pairing one elevator ride with one stairs or escalator leg. Callers use it
/// only when nothing simpler reaches the destination: the original prices any ride that needs
/// a transfer far above a direct one (3000 against 640, plus height), so this serves floors
/// nothing else reaches without rerouting anyone who already has a direct way in.
fn choose_walkway_transfer(
    origin: Endpoint,
    destination: Endpoint,
    elevators: &[ElevatorShaft],
    facilities: &[Facility],
    floors: &[GridPosition],
) -> Option<(TravelMode, f32)> {
    walkway_transfer_candidates(
        origin,
        destination,
        elevators,
        facilities,
        floors,
        &lobby_floors(facilities),
    )
    .into_iter()
    .min_by(|left, right| left.1.total_cmp(&right.1))
}

fn lobby_floors(facilities: &[Facility]) -> Vec<i16> {
    let mut floors: Vec<i16> = facilities
        .iter()
        .filter(|facility| facility.kind == FacilityKind::Lobby)
        .map(|facility| facility.position.floor)
        .collect();
    floors.sort_unstable();
    floors.dedup();
    floors
}

fn choose_route_scored_for(
    origin: Endpoint,
    destination: Endpoint,
    elevators: &[ElevatorShaft],
    facilities: &[Facility],
    floors: &[GridPosition],
    access: ElevatorAccess,
) -> Option<(TravelMode, f32)> {
    if origin.floor == destination.floor {
        return horizontal_floor_path(origin.floor, origin.x, destination.x, floors)
            .then_some((TravelMode::SameFloor, (origin.x - destination.x).abs()));
    }

    let flights = (floor_ordinal(destination.floor) - floor_ordinal(origin.floor)).unsigned_abs();
    let mut candidates = Vec::new();

    // A continuous column may contain several individual stair/escalator
    // flights. Their perceived cost rises quadratically, so people prefer a
    // nearby lift as the climb grows instead of treating every staircase as
    // equally attractive.
    if access == ElevatorAccess::Public {
        for (kind, x) in connected_vertical_walkways(origin.floor, destination.floor, facilities) {
            if let Some(climb) = walkway_climb_cost(kind, flights)
                && horizontal_floor_path(origin.floor, origin.x, x, floors)
                && horizontal_floor_path(destination.floor, x, destination.x, floors)
            {
                let score = (origin.x - x).abs() + (destination.x - x).abs() + climb;
                candidates.push((TravelMode::Stairs { x }, score));
            }
        }
    }

    for shaft in elevators.iter().filter(|shaft| {
        let x = shaft_center(shaft);
        elevator_accepts(shaft.kind, access)
            && shaft_stops_at(shaft, origin.floor, facilities)
            && shaft_stops_at(shaft, destination.floor, facilities)
            && lobby_endpoint_connects_to_shaft(origin, shaft, facilities)
            && lobby_endpoint_connects_to_shaft(destination, shaft, facilities)
            && horizontal_floor_path(origin.floor, origin.x, x, floors)
            && horizontal_floor_path(destination.floor, x, destination.x, floors)
    }) {
        let x = shaft_center(shaft);
        candidates.push((
            TravelMode::Elevator {
                shaft_id: shaft.id,
                x,
            },
            shaft_distance(shaft, origin, destination) + 12.0 + flights as f32 * 0.5,
        ));
    }

    // Housekeepers walk on their current floor and take one direct service
    // elevator between floors. They never use passenger/express cars, stairs,
    // escalators, or public sky-lobby transfers.
    if access == ElevatorAccess::Housekeeping {
        return candidates
            .into_iter()
            .min_by(|left, right| left.1.total_cmp(&right.1));
    }

    let lobby_floors = lobby_floors(facilities);
    for first in elevators.iter().filter(|shaft| {
        let x = shaft_center(shaft);
        elevator_accepts(shaft.kind, access)
            && shaft_stops_at(shaft, origin.floor, facilities)
            && lobby_endpoint_connects_to_shaft(origin, shaft, facilities)
            && horizontal_floor_path(origin.floor, origin.x, x, floors)
    }) {
        for second in elevators.iter().filter(|second| {
            let x = shaft_center(second);
            first.id != second.id
                && second.kind != FacilityKind::ServiceElevator
                && shaft_stops_at(second, destination.floor, facilities)
                && lobby_endpoint_connects_to_shaft(destination, second, facilities)
                && horizontal_floor_path(destination.floor, x, destination.x, floors)
        }) {
            let first_x = shaft_center(first);
            let second_x = shaft_center(second);
            for transfer_floor in lobby_floors.iter().copied().filter(|floor| {
                shaft_stops_at(first, *floor, facilities)
                    && shaft_stops_at(second, *floor, facilities)
                    && lobby_connects_shafts(*floor, first, second, facilities)
            }) {
                let first_flights =
                    (floor_ordinal(transfer_floor) - floor_ordinal(origin.floor)).unsigned_abs();
                let second_flights = (floor_ordinal(destination.floor)
                    - floor_ordinal(transfer_floor))
                .unsigned_abs();
                let score = (origin.x - first_x).abs()
                    + (first_x - second_x).abs()
                    + (destination.x - second_x).abs()
                    + TRANSFER_PENALTY
                    + (first_flights + second_flights) as f32 * 0.5
                    + second_flights as f32 * 0.01;
                candidates.push((
                    TravelMode::ElevatorTransfer {
                        first_id: first.id,
                        first_x,
                        transfer_floor,
                        second_id: second.id,
                        second_x,
                    },
                    score,
                ));
            }
        }
    }

    candidates
        .into_iter()
        .min_by(|left, right| left.1.total_cmp(&right.1))
}

/// The perceived cost of climbing `flights` on one stairs or escalator column, or `None` past
/// the longest climb people accept on it.
fn walkway_climb_cost(kind: FacilityKind, flights: u16) -> Option<f32> {
    let (limit, flight_penalty) = if kind == FacilityKind::Escalator {
        (MAX_ESCALATOR_FLIGHTS, 3.0)
    } else {
        (MAX_STAIR_FLIGHTS, 6.0)
    };
    (flights <= limit).then(|| f32::from(flights * flights) * flight_penalty)
}

fn flights_between(from: i16, to: i16) -> u16 {
    (floor_ordinal(to) - floor_ordinal(from)).unsigned_abs()
}

/// Routes that pair one elevator ride with one stairs or escalator leg, changing between them
/// at a lobby that touches both. The original routes one leg at a time and treats a floor a
/// short climb from a lobby as reachable through that lobby's elevators (tower-together
/// `specs/ROUTING.md`, "Lobby Local Access Ranges"), so a floor beside a sky lobby is served
/// by the express that stops there.
fn walkway_transfer_candidates(
    origin: Endpoint,
    destination: Endpoint,
    elevators: &[ElevatorShaft],
    facilities: &[Facility],
    floors: &[GridPosition],
    lobby_floors: &[i16],
) -> Vec<(TravelMode, f32)> {
    let longest_climb = MAX_STAIR_FLIGHTS.max(MAX_ESCALATOR_FLIGHTS);
    let public_shafts = || {
        elevators
            .iter()
            .filter(|shaft| elevator_accepts(shaft.kind, ElevatorAccess::Public))
    };
    let mut candidates = Vec::new();
    for &transfer_floor in lobby_floors {
        if transfer_floor == origin.floor || transfer_floor == destination.floor {
            continue;
        }

        let climb_flights = flights_between(transfer_floor, destination.floor);
        let ride_flights = flights_between(origin.floor, transfer_floor);
        let walkways = if climb_flights <= longest_climb {
            connected_vertical_walkways(transfer_floor, destination.floor, facilities)
        } else {
            Vec::new()
        };
        for (kind, walkway_x) in walkways {
            let Some(climb) = walkway_climb_cost(kind, climb_flights) else {
                continue;
            };
            if !horizontal_floor_path(destination.floor, walkway_x, destination.x, floors) {
                continue;
            }
            for shaft in public_shafts().filter(|shaft| {
                let shaft_x = shaft_center(shaft);
                shaft_stops_at(shaft, origin.floor, facilities)
                    && shaft_stops_at(shaft, transfer_floor, facilities)
                    && lobby_endpoint_connects_to_shaft(origin, shaft, facilities)
                    && horizontal_floor_path(origin.floor, origin.x, shaft_x, floors)
                    && lobby_connects_shaft_and_walkway(
                        transfer_floor,
                        shaft,
                        kind,
                        walkway_x,
                        facilities,
                    )
                    && horizontal_floor_path(transfer_floor, shaft_x, walkway_x, floors)
            }) {
                let shaft_x = shaft_center(shaft);
                let score = (origin.x - shaft_x).abs()
                    + (shaft_x - walkway_x).abs()
                    + (destination.x - walkway_x).abs()
                    + TRANSFER_PENALTY
                    + f32::from(ride_flights) * 0.5
                    + climb;
                candidates.push((
                    TravelMode::ElevatorThenWalkway {
                        shaft_id: shaft.id,
                        shaft_x,
                        transfer_floor,
                        walkway_x,
                    },
                    score,
                ));
            }
        }

        let climb_flights = flights_between(origin.floor, transfer_floor);
        let ride_flights = flights_between(transfer_floor, destination.floor);
        let walkways = if climb_flights <= longest_climb {
            connected_vertical_walkways(origin.floor, transfer_floor, facilities)
        } else {
            Vec::new()
        };
        for (kind, walkway_x) in walkways {
            let Some(climb) = walkway_climb_cost(kind, climb_flights) else {
                continue;
            };
            if !horizontal_floor_path(origin.floor, origin.x, walkway_x, floors) {
                continue;
            }
            for shaft in public_shafts().filter(|shaft| {
                let shaft_x = shaft_center(shaft);
                shaft_stops_at(shaft, transfer_floor, facilities)
                    && shaft_stops_at(shaft, destination.floor, facilities)
                    && lobby_endpoint_connects_to_shaft(destination, shaft, facilities)
                    && horizontal_floor_path(destination.floor, shaft_x, destination.x, floors)
                    && lobby_connects_shaft_and_walkway(
                        transfer_floor,
                        shaft,
                        kind,
                        walkway_x,
                        facilities,
                    )
                    && horizontal_floor_path(transfer_floor, walkway_x, shaft_x, floors)
            }) {
                let shaft_x = shaft_center(shaft);
                let score = (origin.x - walkway_x).abs()
                    + (walkway_x - shaft_x).abs()
                    + (destination.x - shaft_x).abs()
                    + TRANSFER_PENALTY
                    + f32::from(ride_flights) * 0.5
                    + climb;
                candidates.push((
                    TravelMode::WalkwayThenElevator {
                        walkway_x,
                        transfer_floor,
                        shaft_id: shaft.id,
                        shaft_x,
                    },
                    score,
                ));
            }
        }
    }
    candidates
}

/// Whether one lobby on `floor` touches both the shaft and the stairs or escalator column
/// centred on `walkway_x`, so people can change between them without leaving the lobby.
fn lobby_connects_shaft_and_walkway(
    floor: i16,
    shaft: &ElevatorShaft,
    walkway_kind: FacilityKind,
    walkway_x: f32,
    facilities: &[Facility],
) -> bool {
    let half_width = f32::from(walkway_kind.spec().width) * 0.5;
    lobby_components(floor, facilities)
        .into_iter()
        .any(|component| {
            component_touches_shaft(component, shaft)
                && f32::from(component.0) <= walkway_x + half_width
                && f32::from(component.1) >= walkway_x - half_width
        })
}

fn elevator_accepts(kind: FacilityKind, access: ElevatorAccess) -> bool {
    match access {
        ElevatorAccess::Public => kind != FacilityKind::ServiceElevator,
        ElevatorAccess::Housekeeping => kind == FacilityKind::ServiceElevator,
    }
}

fn shaft_center(shaft: &ElevatorShaft) -> f32 {
    f32::from(shaft.x) + f32::from(shaft.kind.spec().width) * 0.5
}

fn horizontal_floor_path(floor: i16, from_x: f32, to_x: f32, floors: &[GridPosition]) -> bool {
    // Unit routing tests that focus purely on transport may omit structural
    // floor data. Live tower routing always supplies the actual floor cells.
    if floors.is_empty() {
        return true;
    }
    let start = from_x.min(to_x).floor().max(0.0) as u16;
    let end = from_x.max(to_x).floor().max(0.0) as u16;
    // One pass marking covered columns, instead of a linear `contains` scan per column. Every
    // route search walks this check several times; on a large tower the per-column scans of
    // thousands of floor cells made a full route rebuild take seconds.
    let mut uncovered = usize::from(end - start) + 1;
    let mut covered = vec![false; uncovered];
    for cell in floors {
        if cell.floor == floor && (start..=end).contains(&cell.x) {
            let column = &mut covered[usize::from(cell.x - start)];
            if !*column {
                *column = true;
                uncovered -= 1;
                if uncovered == 0 {
                    return true;
                }
            }
        }
    }
    false
}

fn shaft_distance(shaft: &ElevatorShaft, origin: Endpoint, destination: Endpoint) -> f32 {
    let x = f32::from(shaft.x) + f32::from(shaft.kind.spec().width) * 0.5;
    (origin.x - x).abs() + (destination.x - x).abs()
}

fn shaft_stops_at(shaft: &ElevatorShaft, floor: i16, facilities: &[Facility]) -> bool {
    if !shaft.served_floors.contains(&floor)
        || floor_is_inside_multistory_lobby(floor, shaft.x, shaft.kind.spec().width, facilities)
    {
        return false;
    }
    if shaft.kind != FacilityKind::ExpressElevator || floor < 1 {
        return true;
    }

    // Express cars stop above ground only at a real ground/sky lobby that
    // physically touches the shaft. A bare floor at a fifteenth-storey mark
    // is not a stop in the original game.
    (floor == 1 || floor.rem_euclid(15) == 0)
        && lobby_components(floor, facilities)
            .into_iter()
            .any(|component| component_touches_shaft(component, shaft))
}

fn landing_floors(
    kind: FacilityKind,
    x: u16,
    served_floors: &[i16],
    facilities: &[Facility],
) -> Vec<i16> {
    let mut floors = served_floors
        .iter()
        .copied()
        .filter(|floor| !floor_is_inside_multistory_lobby(*floor, x, kind.spec().width, facilities))
        .collect::<Vec<_>>();
    if floors.is_empty() {
        // A shaft may be built top-down through a grand lobby before its base
        // landing is added. Keep the temporary car inside the physical shaft;
        // routing still rejects these floors as passenger stops.
        floors.extend_from_slice(served_floors);
    }
    floors
}

fn normalize_elevator_home_floors(elevators: &mut [ElevatorShaft], facilities: &[Facility]) {
    for shaft in elevators {
        let valid_floors = landing_floors(shaft.kind, shaft.x, &shaft.served_floors, facilities);
        let default_floor = default_home_floor(&valid_floors);
        for car in &mut shaft.cars {
            if !valid_floors.contains(&car.home_floor) {
                car.home_floor = default_floor;
            }
        }
    }
}

/// Keeps only the passengers who are still riding the car's shaft. Demolishing a tenant removes
/// its people even mid-ride, and an id left behind would hold a car place for good, since only
/// a rider's own arrival frees it. The original's teardown likewise removes a cancelled
/// request from every active route slot. Older saves can already carry such ids, and loading
/// syncs, so this also repairs them.
fn release_departed_passengers(elevators: &mut [ElevatorShaft], people: &[Person]) {
    let riders = people
        .iter()
        .filter_map(|person| match person.activity {
            PersonActivity::RidingElevator { shaft_id } => Some((shaft_id, person.id)),
            _ => None,
        })
        .collect::<HashSet<_>>();
    for shaft in elevators {
        let shaft_id = shaft.id;
        for car in &mut shaft.cars {
            car.passengers
                .retain(|id| riders.contains(&(shaft_id, *id)));
        }
    }
}

fn floor_is_inside_multistory_lobby(
    floor: i16,
    transport_x: u16,
    transport_width: u16,
    facilities: &[Facility],
) -> bool {
    let transport_end = transport_x.saturating_add(transport_width);
    facilities.iter().any(|lobby| {
        lobby.kind == FacilityKind::Lobby
            && lobby.stories > 1
            && floor > lobby.position.floor
            && floor < lobby.position.floor + i16::from(lobby.stories) - 1
            && lobby.position.x <= transport_end
            && lobby.end_x() >= transport_x
    })
}

pub fn elevator_landing_is_valid(
    kind: FacilityKind,
    x: u16,
    floor: i16,
    facilities: &[Facility],
) -> bool {
    matches!(
        kind,
        FacilityKind::Elevator | FacilityKind::ServiceElevator | FacilityKind::ExpressElevator
    ) && !floor_is_inside_multistory_lobby(floor, x, kind.spec().width, facilities)
}

fn lobby_endpoint_connects_to_shaft(
    endpoint: Endpoint,
    shaft: &ElevatorShaft,
    facilities: &[Facility],
) -> bool {
    let Some(lobby) = facilities.iter().find(|facility| {
        facility.id == endpoint.facility_id && facility.kind == FacilityKind::Lobby
    }) else {
        return true;
    };
    lobby_components(endpoint.floor, facilities)
        .into_iter()
        .any(|component| {
            component.0 <= lobby.position.x
                && lobby.position.x < component.1
                && component_touches_shaft(component, shaft)
        })
}

fn lobby_connects_shafts(
    floor: i16,
    first: &ElevatorShaft,
    second: &ElevatorShaft,
    facilities: &[Facility],
) -> bool {
    lobby_components(floor, facilities)
        .into_iter()
        .any(|component| {
            component_touches_shaft(component, first) && component_touches_shaft(component, second)
        })
}

/// Returns half-open horizontal runs of contiguous lobby slices. Elevator
/// shafts may be painted over lobby artwork, so touching or overlapping a run
/// both count as a connection.
fn lobby_components(floor: i16, facilities: &[Facility]) -> Vec<(u16, u16)> {
    let mut columns = facilities
        .iter()
        .filter(|facility| facility.kind == FacilityKind::Lobby && facility.position.floor == floor)
        .map(|facility| facility.position.x)
        .collect::<Vec<_>>();
    columns.sort_unstable();
    columns.dedup();

    let mut components = Vec::new();
    let Some(mut start) = columns.first().copied() else {
        return components;
    };
    let mut end = start + 1;
    for column in columns.into_iter().skip(1) {
        if column == end {
            end += 1;
        } else {
            components.push((start, end));
            start = column;
            end = column + 1;
        }
    }
    components.push((start, end));
    components
}

fn component_touches_shaft(component: (u16, u16), shaft: &ElevatorShaft) -> bool {
    let shaft_start = shaft.x;
    let shaft_end = shaft.x.saturating_add(shaft.kind.spec().width);
    component.0 <= shaft_end && component.1 >= shaft_start
}

fn connected_vertical_walkways(
    origin_floor: i16,
    destination_floor: i16,
    facilities: &[Facility],
) -> Vec<(FacilityKind, f32)> {
    let mut connected_walkways = Vec::new();
    for kind in [FacilityKind::Escalator, FacilityKind::Stairs] {
        let mut columns: Vec<u16> = facilities
            .iter()
            .filter(|facility| facility.kind == kind)
            .map(|facility| facility.position.x)
            .collect();
        columns.sort_unstable();
        columns.dedup();
        for x in columns {
            if floor_is_inside_multistory_lobby(origin_floor, x, kind.spec().width, facilities)
                || floor_is_inside_multistory_lobby(
                    destination_floor,
                    x,
                    kind.spec().width,
                    facilities,
                )
            {
                continue;
            }
            let mut ordinal = floor_ordinal(origin_floor);
            let target = floor_ordinal(destination_floor);
            let direction = if target > ordinal { 1 } else { -1 };
            let mut connected = true;
            while ordinal != target {
                let base_ordinal = if direction > 0 { ordinal } else { ordinal - 1 };
                if !facilities.iter().any(|facility| {
                    facility.kind == kind
                        && facility.position.x == x
                        && walkway_covers_flight(facility, base_ordinal, facilities)
                }) {
                    connected = false;
                    break;
                }
                ordinal += direction;
            }
            if connected {
                connected_walkways.push((kind, f32::from(x) + f32::from(kind.spec().width) * 0.5));
            }
        }
    }
    connected_walkways
}

fn walkway_covers_flight(
    walkway: &Facility,
    flight_base_ordinal: i16,
    facilities: &[Facility],
) -> bool {
    let walkway_base = floor_ordinal(walkway.position.floor);
    let flights = vertical_walkway_story_span(walkway, facilities).saturating_sub(1) as i16;
    flight_base_ordinal >= walkway_base && flight_base_ordinal < walkway_base + flights
}

/// Rendered and routed height of stairs or an escalator. A transport placed
/// in the bottom of a grand lobby expands through the entire open lobby,
/// rather than requiring an unreachable landing in its middle story.
pub fn vertical_walkway_story_span(walkway: &Facility, facilities: &[Facility]) -> u16 {
    vertical_walkway_story_span_at(walkway.kind, walkway.position, facilities)
}

pub fn vertical_walkway_story_span_at(
    kind: FacilityKind,
    position: GridPosition,
    facilities: &[Facility],
) -> u16 {
    let normal_height = kind.spec().height;
    if !matches!(kind, FacilityKind::Stairs | FacilityKind::Escalator) {
        return normal_height;
    }
    let end_x = position.x.saturating_add(kind.spec().width);
    facilities
        .iter()
        .filter(|lobby| {
            lobby.kind == FacilityKind::Lobby
                && lobby.position.floor == position.floor
                && lobby.stories > 1
                && position.x < lobby.end_x()
                && end_x > lobby.position.x
        })
        .map(|lobby| u16::from(lobby.stories))
        .max()
        .unwrap_or(normal_height)
        .max(normal_height)
}

fn start_trip(person: &mut Person) {
    person.activity = PersonActivity::Walking;
    person.wait_seconds = 0.0;
    person.mood = PersonMood::Calm;
    person.route_leg = 0;
}

fn arrive_at_facility(person: &mut Person) {
    person.activity = PersonActivity::Visiting;
    person.visit_remaining = if person.going_home {
        0.0
    } else {
        person.visit_duration
    };
    person.current_floor = trip_destination(person).floor;
    person.floor_position = f32::from(floor_ordinal(person.current_floor));
}

fn trip_destination(person: &Person) -> Endpoint {
    if person.going_home {
        person.origin
    } else {
        person.destination
    }
}

fn desired_elevator_floor(person: &Person) -> i16 {
    match person.route {
        TravelMode::ElevatorTransfer { transfer_floor, .. }
        | TravelMode::ElevatorThenWalkway { transfer_floor, .. }
            if person.route_leg == 0 =>
        {
            transfer_floor
        }
        _ => trip_destination(person).floor,
    }
}

/// The floor a stairs or escalator leg ends on: the transfer lobby when the walkway comes
/// first, otherwise the destination floor.
fn walkway_leg_end_floor(person: &Person) -> i16 {
    match person.route {
        TravelMode::WalkwayThenElevator { transfer_floor, .. } if person.route_leg == 0 => {
            transfer_floor
        }
        _ => trip_destination(person).floor,
    }
}

/// Ends the leg a person has just ridden or climbed. A two-leg route walks on to its second
/// transport; otherwise the person walks to the destination.
fn finish_leg(person: &mut Person) {
    let second_leg_follows = person.route_leg == 0
        && matches!(
            person.route,
            TravelMode::ElevatorTransfer { .. }
                | TravelMode::ElevatorThenWalkway { .. }
                | TravelMode::WalkwayThenElevator { .. }
        );
    if second_leg_follows {
        person.route_leg = 1;
        person.walking_from_transport = false;
    } else {
        person.walking_from_transport = true;
    }
}

fn reverse_route(route: TravelMode) -> TravelMode {
    match route {
        TravelMode::ElevatorTransfer {
            first_id,
            first_x,
            transfer_floor,
            second_id,
            second_x,
        } => TravelMode::ElevatorTransfer {
            first_id: second_id,
            first_x: second_x,
            transfer_floor,
            second_id: first_id,
            second_x: first_x,
        },
        TravelMode::ElevatorThenWalkway {
            shaft_id,
            shaft_x,
            transfer_floor,
            walkway_x,
        } => TravelMode::WalkwayThenElevator {
            walkway_x,
            transfer_floor,
            shaft_id,
            shaft_x,
        },
        TravelMode::WalkwayThenElevator {
            walkway_x,
            transfer_floor,
            shaft_id,
            shaft_x,
        } => TravelMode::ElevatorThenWalkway {
            shaft_id,
            shaft_x,
            transfer_floor,
            walkway_x,
        },
        other => other,
    }
}

fn endpoint(facility: &Facility) -> Endpoint {
    Endpoint {
        facility_id: facility.id,
        x: f32::from(facility.position.x) + f32::from(facility.kind.spec().width) * 0.5,
        floor: facility.position.floor,
    }
}

fn facility_top_endpoint(facility: &Facility) -> Endpoint {
    let top_ordinal = floor_ordinal(facility.position.floor)
        + facility.kind.spec().height.saturating_sub(1) as i16;
    Endpoint {
        facility_id: facility.id,
        x: f32::from(facility.position.x) + f32::from(facility.kind.spec().width) * 0.5,
        floor: floor_from_ordinal(top_ordinal),
    }
}

const fn commercial_patronage_target(kind: FacilityKind) -> Option<u32> {
    match kind {
        FacilityKind::FastFood | FacilityKind::Restaurant => Some(35),
        FacilityKind::Shop => Some(25),
        FacilityKind::Cinema => Some(20),
        FacilityKind::PartyHall => Some(50),
        _ => None,
    }
}

const fn commercial_patronage_initial(kind: FacilityKind) -> u32 {
    match kind {
        // These attractions open with a complete event/audience rather than
        // slowly accumulating ordinary retail customers.
        FacilityKind::Cinema => 120,
        FacilityKind::PartyHall => 50,
        FacilityKind::FastFood | FacilityKind::Restaurant | FacilityKind::Shop => 4,
        _ => 0,
    }
}

fn exterior_spawn_x(tower: &Tower, seed: u64) -> f32 {
    let mut occupied_extents = tower
        .facilities()
        .iter()
        .filter(|facility| facility.position.floor == 1)
        .map(|facility| (facility.position.x, facility.end_x()));
    let occupied_bounds = occupied_extents.next().map(|first| {
        occupied_extents.fold(first, |(left, right), (start, end)| {
            (left.min(start), right.max(end))
        })
    });
    let mut first_floor_columns = tower
        .floors()
        .iter()
        .filter(|position| position.floor == 1)
        .map(|position| position.x);
    let floor_bounds = first_floor_columns.next().map(|first_column| {
        first_floor_columns.fold((first_column, first_column + 1), |(left, right), column| {
            (left.min(column), right.max(column + 1))
        })
    });
    let Some((left, right)) = occupied_bounds.or(floor_bounds) else {
        return if seed & 1 == 0 {
            -12.0
        } else {
            f32::from(tower.width()) + 12.0
        };
    };
    if seed & 1 == 0 {
        f32::from(left) - 12.0
    } else {
        f32::from(right) + 12.0
    }
}

fn is_person_destination(kind: FacilityKind) -> bool {
    matches!(
        kind,
        FacilityKind::Office
            | FacilityKind::HotelSingle
            | FacilityKind::HotelTwin
            | FacilityKind::HotelSuite
            | FacilityKind::Restaurant
            | FacilityKind::FastFood
            | FacilityKind::Shop
            | FacilityKind::Cinema
            | FacilityKind::PartyHall
            | FacilityKind::Medical
    )
}

pub fn facility_is_open(kind: FacilityKind, minute: u16) -> bool {
    match kind {
        FacilityKind::Office => (9 * 60..17 * 60).contains(&minute),
        FacilityKind::FastFood | FacilityKind::Shop => (10 * 60..21 * 60).contains(&minute),
        FacilityKind::Restaurant => (17 * 60..23 * 60).contains(&minute),
        FacilityKind::Cinema => (13 * 60..20 * 60).contains(&minute),
        FacilityKind::PartyHall => (17 * 60..23 * 60).contains(&minute),
        FacilityKind::Medical => (8 * 60..18 * 60).contains(&minute),
        _ => true,
    }
}

pub fn facility_is_open_at(kind: FacilityKind, clock: Clock) -> bool {
    if kind == FacilityKind::Office && is_weekend(clock) {
        return false;
    }
    match kind {
        FacilityKind::HotelSingle | FacilityKind::HotelTwin | FacilityKind::HotelSuite => {
            (16 * 60..22 * 60).contains(&clock.minute_of_day)
        }
        _ => facility_is_open(kind, clock.minute_of_day),
    }
}

pub const fn is_weekend(clock: Clock) -> bool {
    (clock.day.saturating_sub(1) % 7) + 1 >= 6
}

fn visit_duration_seconds(kind: FacilityKind) -> f32 {
    match kind {
        // At normal speed the simulation advances two game minutes per real
        // second. These stays therefore track the guide's daily schedules.
        FacilityKind::Office => 300.0,
        FacilityKind::Condo => 360.0,
        FacilityKind::HotelSingle | FacilityKind::HotelTwin | FacilityKind::HotelSuite => 240.0,
        FacilityKind::Restaurant => 35.0,
        FacilityKind::FastFood => 18.0,
        FacilityKind::Shop => 28.0,
        FacilityKind::Cinema | FacilityKind::PartyHall => 60.0,
        FacilityKind::Medical => 24.0,
        _ => 5.0,
    }
}

fn default_home_floor(floors: &[i16]) -> i16 {
    if floors.contains(&1) {
        1
    } else {
        floors.first().copied().unwrap_or(1)
    }
}

pub fn floor_ordinal(floor: i16) -> i16 {
    if floor > 0 { floor - 1 } else { floor }
}

pub fn floor_from_ordinal(ordinal: i16) -> i16 {
    if ordinal >= 0 { ordinal + 1 } else { ordinal }
}

pub fn schedule_period(minute_of_day: u16) -> usize {
    match minute_of_day {
        0..=419 => 5,
        420..=719 => 0,
        720..=779 => 1,
        780..=1019 => 2,
        1020..=1259 => 3,
        _ => 4,
    }
}

pub fn schedule_index(clock: Clock) -> usize {
    schedule_period(clock.minute_of_day) + if is_weekend(clock) { 6 } else { 0 }
}

fn move_toward(value: f32, target: f32, amount: f32) -> f32 {
    if value < target {
        (value + amount).min(target)
    } else {
        (value - amount).max(target)
    }
}

fn mix64(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn next_spawn_interval(star_rating: u8, seed: u64) -> f32 {
    let rating = f32::from(star_rating.clamp(1, 5));
    let random_fraction = (mix64(seed.rotate_left(11)) % 1_000) as f32 / 1_000.0;
    // More highly rated towers attract more people, while a broad deterministic
    // jitter prevents the entrances and elevators from receiving metronomic
    // batches. At one star this is roughly 3.6-8.5 seconds per arrival; at five
    // stars it is about 0.7-1.7 seconds.
    (5.5 / rating) * (0.65 + random_fraction * 0.9)
}

#[cfg(test)]
mod discovery;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FacilityOccupancy, GridPosition};

    fn test_shaft(id: u64, kind: FacilityKind, x: u16, served_floors: &[i16]) -> ElevatorShaft {
        ElevatorShaft {
            id,
            kind,
            x,
            served_floors: served_floors.to_vec(),
            cars: Vec::new(),
            schedules: [ElevatorSchedule::default(); 12],
            visible: true,
        }
    }

    fn test_facility(id: u64, kind: FacilityKind, x: u16, floor: i16) -> Facility {
        Facility {
            id,
            kind,
            position: GridPosition { x, floor },
            occupancy: FacilityOccupancy::NotApplicable,
            price_level: 1,
            stories: kind.spec().height as u8,
        }
    }

    fn place_floor_run(tower: &mut Tower, floor: i16, width: u16) {
        for level in 1..=floor {
            for x in 0..width {
                if !tower.has_floor(GridPosition { x, floor: level }) {
                    tower.place_floor(GridPosition { x, floor: level }).unwrap();
                }
            }
        }
    }

    fn traffic_tower(with_upper_elevator: bool) -> Tower {
        let mut tower = Tower::new(64, 10_000_000);
        place_floor_run(&mut tower, 2, 64);
        tower
            .place(FacilityKind::Office, GridPosition { x: 0, floor: 2 })
            .unwrap();
        tower
            .place(FacilityKind::FastFood, GridPosition { x: 32, floor: 1 })
            .unwrap();
        tower
            .place(FacilityKind::Lobby, GridPosition { x: 24, floor: 1 })
            .unwrap();
        tower
            .place(FacilityKind::Elevator, GridPosition { x: 20, floor: 1 })
            .unwrap();
        if with_upper_elevator {
            tower
                .place(FacilityKind::Elevator, GridPosition { x: 20, floor: 2 })
                .unwrap();
        }
        tower.advance_time(60.0);
        tower
    }

    #[test]
    fn floor_ordinals_skip_zero_without_adding_travel_distance() {
        assert_eq!(floor_ordinal(-1), -1);
        assert_eq!(floor_ordinal(1), 0);
        assert_eq!(floor_ordinal(2), 1);
        assert_eq!(floor_from_ordinal(-1), -1);
        assert_eq!(floor_from_ordinal(0), 1);
    }

    #[test]
    fn contiguous_elevator_segments_form_one_shaft() {
        let tower = traffic_tower(true);
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        assert_eq!(traffic.elevators.len(), 1);
        assert_eq!(traffic.elevators[0].served_floors, vec![1, 2]);
        assert_eq!(traffic.elevators[0].cars.len(), 1);
        assert_eq!(traffic.elevators[0].cars[0].home_floor, 1);
    }

    #[test]
    fn parking_coverage_crosses_only_three_empty_units_and_stops_at_objects() {
        let facilities = vec![
            test_facility(1, FacilityKind::Lobby, 0, 1),
            test_facility(2, FacilityKind::Ramp, 0, -1),
            test_facility(3, FacilityKind::Parking, 19, -1),
            test_facility(4, FacilityKind::Parking, 26, -1),
            test_facility(5, FacilityKind::Office, 31, -1),
            test_facility(6, FacilityKind::Parking, 41, -1),
        ];

        assert_eq!(connected_parking_spaces(&facilities), HashSet::from([3, 4]));
    }

    #[test]
    fn parking_requests_follow_the_original_office_column_and_suite_rules() {
        assert!(tenant_requests_parking(&test_facility(
            1,
            FacilityKind::Office,
            3,
            1
        )));
        assert!(!tenant_requests_parking(&test_facility(
            2,
            FacilityKind::Office,
            4,
            1
        )));
        assert!(tenant_requests_parking(&test_facility(
            3,
            FacilityKind::HotelSuite,
            4,
            20
        )));
        assert!(!tenant_requests_parking(&test_facility(
            4,
            FacilityKind::HotelTwin,
            4,
            20
        )));
    }

    #[test]
    fn a_parking_space_is_reserved_by_one_driver_until_that_trip_ends() {
        let mut tower = Tower::new(64, 10_000_000);
        for floor in [-1, 1] {
            for x in 0..64 {
                tower.place_floor(GridPosition { x, floor }).unwrap();
            }
        }
        tower
            .place(FacilityKind::Lobby, GridPosition { x: 0, floor: 1 })
            .unwrap();
        let office_id = tower
            .place(FacilityKind::Office, GridPosition { x: 3, floor: 1 })
            .unwrap();
        tower
            .place(FacilityKind::Ramp, GridPosition { x: 0, floor: -1 })
            .unwrap();
        let parking_id = tower
            .place(FacilityKind::Parking, GridPosition { x: 16, floor: -1 })
            .unwrap();
        tower
            .place(FacilityKind::Stairs, GridPosition { x: 20, floor: -1 })
            .unwrap();
        // This test exercises the parking request path directly; final tower
        // status keeps the normal three-star gate open without manufacturing
        // hundreds of unrelated occupants.
        tower.tower_status_awarded = true;

        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        let office = tower
            .facilities()
            .iter()
            .find(|facility| facility.id == office_id)
            .unwrap();
        let destination = endpoint(office);
        let lobby_route = *traffic.routes_from_lobby.get(&office_id).unwrap();
        let (origin, route) = traffic.arrival_origin(&tower, office, destination, lobby_route, 17);
        assert_eq!(origin.facility_id, parking_id);

        traffic.people.push(Person {
            id: 1,
            role: PersonRole::Salesman,
            home_facility_id: parking_id,
            destination_facility_id: office_id,
            x: origin.x,
            floor_position: f32::from(floor_ordinal(origin.floor)),
            current_floor: origin.floor,
            mood: PersonMood::Calm,
            activity: PersonActivity::Walking,
            wait_seconds: 0.0,
            origin,
            destination,
            route,
            going_home: false,
            walking_from_transport: false,
            visit_remaining: 0.0,
            visit_duration: 1.0,
            stair_progress: 0.0,
            route_leg: 0,
        });

        let (second_origin, _) =
            traffic.arrival_origin(&tower, office, destination, lobby_route, 18);
        assert_eq!(second_origin.facility_id, 0);

        traffic.people.clear();
        traffic.connected_parking.clear();
        let (unserved_origin, _) =
            traffic.arrival_origin(&tower, office, destination, lobby_route, 19);
        assert_eq!(unserved_origin.facility_id, 0);
        assert!(traffic.take_parking_demand_warning());
        assert!(!traffic.take_parking_demand_warning());
    }

    #[test]
    fn lobby_routes_are_cached_until_tower_topology_changes() {
        let mut tower = traffic_tower(true);
        let office_id = tower
            .facilities()
            .iter()
            .find(|facility| facility.kind == FacilityKind::Office)
            .unwrap()
            .id;
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        assert!(matches!(
            traffic.routes_from_lobby.get(&office_id),
            Some(TravelMode::Elevator { .. })
        ));

        // A sentinel route survives a no-op sync, demonstrating that the
        // costly path search is not repeated on every simulation frame.
        traffic
            .routes_from_lobby
            .insert(office_id, TravelMode::SameFloor);
        traffic.sync_with_tower(&tower);
        assert_eq!(
            traffic.routes_from_lobby.get(&office_id),
            Some(&TravelMode::SameFloor)
        );

        // Adding another tenant computes only that destination. Existing
        // routes stay untouched because corridors and transport did not
        // change.
        let second_office = tower
            .place(FacilityKind::Office, GridPosition { x: 9, floor: 2 })
            .unwrap();
        traffic.sync_with_tower(&tower);
        assert_eq!(
            traffic.routes_from_lobby.get(&office_id),
            Some(&TravelMode::SameFloor)
        );
        assert!(matches!(
            traffic.routes_from_lobby.get(&second_office),
            Some(TravelMode::Elevator { .. })
        ));

        for x in 20..24 {
            tower.place_floor(GridPosition { x, floor: 3 }).unwrap();
        }
        traffic.sync_with_tower_after_addition(&tower);
        assert_eq!(
            traffic.routes_from_lobby.get(&office_id),
            Some(&TravelMode::SameFloor)
        );

        let extension = tower
            .place(FacilityKind::Elevator, GridPosition { x: 20, floor: 3 })
            .unwrap();
        traffic.sync_with_tower_after_addition(&tower);
        assert_eq!(
            traffic.routes_from_lobby.get(&office_id),
            Some(&TravelMode::SameFloor)
        );
        assert_eq!(traffic.elevators[0].served_floors, vec![1, 2, 3]);

        tower.demolish(extension).unwrap();
        traffic.sync_with_tower(&tower);
        assert!(matches!(
            traffic.routes_from_lobby.get(&office_id),
            Some(TravelMode::Elevator { .. })
        ));
    }

    /// The original lobby-route search: one full route search from every first-floor slice.
    fn route_from_first_floor_lobby_exhaustive(
        destination: Endpoint,
        elevators: &[ElevatorShaft],
        facilities: &[Facility],
        floors: &[GridPosition],
    ) -> Option<TravelMode> {
        facilities
            .iter()
            .filter(|facility| facility.kind == FacilityKind::Lobby && facility.position.floor == 1)
            .filter_map(|lobby| {
                choose_route_scored(endpoint(lobby), destination, elevators, facilities, floors)
            })
            .min_by(|left, right| left.1.total_cmp(&right.1))
            .map(|(route, _)| route)
    }

    #[test]
    fn lobby_origin_pruning_matches_the_exhaustive_search() {
        let mut tower = Tower::new(100, 100_000_000);
        place_floor_run(&mut tower, 6, 100);
        // Two separate lobby stretches, so components and gaps both matter.
        for x in (6..28).chain(34..60) {
            tower
                .place(FacilityKind::Lobby, GridPosition { x, floor: 1 })
                .unwrap();
        }
        for floor in 1..=6 {
            for x in [2, 30] {
                tower
                    .place(FacilityKind::Elevator, GridPosition { x, floor })
                    .unwrap();
            }
        }
        for floor in 1..=4 {
            tower
                .place(FacilityKind::Elevator, GridPosition { x: 90, floor })
                .unwrap();
        }
        for floor in 1..=2 {
            tower
                .place(FacilityKind::Stairs, GridPosition { x: 72, floor })
                .unwrap();
        }
        tower
            .place(FacilityKind::Escalator, GridPosition { x: 80, floor: 1 })
            .unwrap();
        tower
            .place(FacilityKind::Shop, GridPosition { x: 60, floor: 1 })
            .unwrap();
        for floor in 2..=6 {
            for x in [8, 20, 36, 48, 60] {
                tower
                    .place(FacilityKind::Office, GridPosition { x, floor })
                    .unwrap();
            }
        }
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);

        let mut routes = Vec::new();
        for facility in tower
            .facilities()
            .iter()
            .filter(|facility| facility.kind.has_tenant_occupancy())
        {
            let destination = endpoint(facility);
            let args = (&traffic.elevators, tower.facilities(), tower.floors());
            let pruned = route_from_first_floor_lobby(destination, args.0, args.1, args.2);
            let exhaustive =
                route_from_first_floor_lobby_exhaustive(destination, args.0, args.1, args.2);
            assert_eq!(pruned, exhaustive, "destination {facility:?}");
            routes.push(pruned);
        }
        // The tower must exercise stairs, the same-floor shop and more than one shaft.
        assert!(routes.contains(&Some(TravelMode::SameFloor)));
        assert!(
            routes
                .iter()
                .any(|route| matches!(route, Some(TravelMode::Stairs { .. })))
        );
        let shafts = routes
            .iter()
            .filter_map(|route| match route {
                Some(TravelMode::Elevator { shaft_id, .. }) => Some(*shaft_id),
                _ => None,
            })
            .collect::<HashSet<_>>();
        assert!(shafts.len() > 1, "routes: {routes:?}");
    }

    #[test]
    fn stair_demolition_rebuilds_only_routes_that_cross_the_removed_flight() {
        let mut tower = Tower::new(64, 10_000_000);
        place_floor_run(&mut tower, 1, 64);
        place_floor_run(&mut tower, 2, 64);
        tower
            .place(FacilityKind::Lobby, GridPosition { x: 0, floor: 1 })
            .unwrap();
        let stairs = tower
            .place(FacilityKind::Stairs, GridPosition { x: 8, floor: 1 })
            .unwrap();
        let lower_office = tower
            .place(FacilityKind::Office, GridPosition { x: 20, floor: 1 })
            .unwrap();
        let upper_office = tower
            .place(FacilityKind::Office, GridPosition { x: 20, floor: 2 })
            .unwrap();

        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        assert!(matches!(
            traffic.routes_from_lobby.get(&upper_office),
            Some(TravelMode::Stairs { .. })
        ));

        // A sentinel on an unrelated route makes an unnecessary global
        // rebuild observable rather than relying on timing in the test.
        let sentinel = TravelMode::Elevator {
            shaft_id: u64::MAX,
            x: 999.0,
        };
        traffic.routes_from_lobby.insert(lower_office, sentinel);

        let removed = tower.demolish(stairs).unwrap();
        traffic.sync_with_tower_after_removal(&tower, &removed);

        assert_eq!(
            traffic.routes_from_lobby.get(&lower_office),
            Some(&sentinel)
        );
        assert!(!traffic.routes_from_lobby.contains_key(&upper_office));
        assert!(traffic.reachable_facilities.contains(&lower_office));
        assert!(!traffic.reachable_facilities.contains(&upper_office));
    }

    #[test]
    fn demolishing_a_tenant_frees_the_car_places_of_its_riders() {
        let mut tower = traffic_tower(true);
        let office = tower
            .facilities()
            .iter()
            .find(|facility| facility.kind == FacilityKind::Office)
            .unwrap()
            .clone();
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        let route = *traffic.routes_from_lobby.get(&office.id).unwrap();
        let TravelMode::Elevator { shaft_id, x } = route else {
            panic!("expected an elevator route, got {route:?}");
        };
        let origin = Endpoint {
            facility_id: 0,
            x,
            floor: 1,
        };
        traffic.people.push(Person {
            id: 1,
            role: PersonRole::Visitor,
            home_facility_id: 0,
            destination_facility_id: office.id,
            x,
            floor_position: 0.5,
            current_floor: 1,
            mood: PersonMood::Calm,
            activity: PersonActivity::RidingElevator { shaft_id },
            wait_seconds: 0.0,
            origin,
            destination: endpoint(&office),
            route,
            going_home: false,
            walking_from_transport: false,
            visit_remaining: 0.0,
            visit_duration: 1.0,
            stair_progress: 0.0,
            route_leg: 0,
        });
        traffic.elevator_mut(shaft_id).unwrap().cars[0]
            .passengers
            .push(1);

        let removed = tower.demolish(office.id).unwrap();
        traffic.sync_with_tower_after_removal(&tower, &removed);

        assert!(traffic.people.is_empty());
        assert!(
            traffic.elevator_mut(shaft_id).unwrap().cars[0]
                .passengers
                .is_empty()
        );
    }

    #[test]
    fn syncing_frees_car_places_held_by_people_who_are_gone() {
        let tower = traffic_tower(true);
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        let shaft_id = traffic.elevators[0].id;
        traffic.elevators[0].cars[0].passengers.push(42);

        // Loading a save syncs without any topology change, which must still repair it.
        traffic.sync_with_tower(&tower);

        assert!(
            traffic.elevator_mut(shaft_id).unwrap().cars[0]
                .passengers
                .is_empty()
        );
    }

    #[test]
    fn basement_floor_demolition_rebuilds_only_routes_crossing_that_cell() {
        let mut tower = Tower::new(64, 10_000_000);
        for x in 8..14 {
            tower.place_floor(GridPosition { x, floor: 1 }).unwrap();
        }
        for x in 8..64 {
            tower.place_floor(GridPosition { x, floor: -1 }).unwrap();
        }
        tower
            .place(FacilityKind::Lobby, GridPosition { x: 12, floor: 1 })
            .unwrap();
        tower
            .place(FacilityKind::Elevator, GridPosition { x: 8, floor: 1 })
            .unwrap();
        tower
            .place(FacilityKind::Elevator, GridPosition { x: 8, floor: -1 })
            .unwrap();
        let near_office = tower
            .place(FacilityKind::Office, GridPosition { x: 16, floor: -1 })
            .unwrap();
        let far_office = tower
            .place(FacilityKind::Office, GridPosition { x: 40, floor: -1 })
            .unwrap();

        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        assert!(traffic.routes_from_lobby.contains_key(&near_office));
        assert!(traffic.routes_from_lobby.contains_key(&far_office));

        // A sentinel makes an accidental global route rebuild observable.
        let sentinel = TravelMode::Elevator {
            shaft_id: u64::MAX,
            x: 10.0,
        };
        traffic.routes_from_lobby.insert(near_office, sentinel);
        let removed = GridPosition { x: 30, floor: -1 };
        tower.demolish_floor(removed).unwrap();
        traffic.sync_with_tower_after_floor_removal(&tower, removed);

        assert_eq!(traffic.routes_from_lobby.get(&near_office), Some(&sentinel));
        assert!(!traffic.routes_from_lobby.contains_key(&far_office));
        assert!(traffic.reachable_facilities.contains(&near_office));
        assert!(!traffic.reachable_facilities.contains(&far_office));
    }

    /// An elevator at x 8 over floors `1..=floors` with an office on every upper floor, synced.
    /// Returns the shaft's segment ids from the bottom up.
    fn shaft_with_offices(floors: i16) -> (Tower, TrafficSimulation, Vec<u64>) {
        let mut tower = Tower::new(32, 100_000_000);
        place_floor_run(&mut tower, floors, 32);
        let segments = (1..=floors)
            .map(|floor| {
                tower
                    .place(FacilityKind::Elevator, GridPosition { x: 8, floor })
                    .unwrap()
            })
            .collect();
        for floor in 2..=floors {
            tower
                .place(FacilityKind::Office, GridPosition { x: 16, floor })
                .unwrap();
        }
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        (tower, traffic, segments)
    }

    /// A visitor on `current_floor` using the first shaft to reach the office on `office_floor`.
    fn shaft_traveller(
        tower: &Tower,
        traffic: &TrafficSimulation,
        id: u64,
        current_floor: i16,
        office_floor: i16,
        activity: PersonActivity,
    ) -> Person {
        let shaft = &traffic.elevators[0];
        let office = tower
            .facilities()
            .iter()
            .find(|facility| {
                facility.kind == FacilityKind::Office && facility.position.floor == office_floor
            })
            .unwrap();
        Person {
            id,
            role: PersonRole::Visitor,
            home_facility_id: 0,
            destination_facility_id: office.id,
            x: shaft_center(shaft),
            floor_position: f32::from(floor_ordinal(current_floor)),
            current_floor,
            mood: PersonMood::Calm,
            activity,
            wait_seconds: 0.0,
            origin: Endpoint {
                facility_id: 0,
                x: 2.0,
                floor: 1,
            },
            destination: endpoint(office),
            route: TravelMode::Elevator {
                shaft_id: shaft.id,
                x: shaft_center(shaft),
            },
            going_home: false,
            walking_from_transport: false,
            visit_remaining: 0.0,
            visit_duration: 1.0,
            stair_progress: 0.0,
            route_leg: 0,
        }
    }

    fn activity_of(traffic: &TrafficSimulation, id: u64) -> PersonActivity {
        traffic
            .people
            .iter()
            .find(|person| person.id == id)
            .unwrap()
            .activity
    }

    #[test]
    fn shortening_a_shaft_abandons_trips_it_can_no_longer_complete() {
        let (mut tower, mut traffic, segments) = shaft_with_offices(6);
        let shaft_id = traffic.elevators[0].id;
        let riding = PersonActivity::RidingElevator { shaft_id };
        let waiting = PersonActivity::WaitingForElevator { shaft_id };
        let rider_to_top = shaft_traveller(&tower, &traffic, 1, 1, 6, riding);
        let mut waiting_on_top = shaft_traveller(&tower, &traffic, 2, 6, 6, waiting);
        waiting_on_top.going_home = true;
        let waiting_for_top = shaft_traveller(&tower, &traffic, 3, 2, 6, waiting);
        let waiting_for_served = shaft_traveller(&tower, &traffic, 4, 2, 4, waiting);
        // Inside the top office; their trip home starts after the shaft is cut.
        let mut visiting_top = shaft_traveller(&tower, &traffic, 5, 6, 6, PersonActivity::Visiting);
        visiting_top.visit_remaining = 1.0;
        traffic.people.extend([
            rider_to_top,
            waiting_on_top,
            waiting_for_top,
            waiting_for_served,
            visiting_top,
        ]);
        let car = &mut traffic.elevators[0].cars[0];
        car.floor_position = 4.5;
        car.direction = ElevatorDirection::Up;
        car.target_floor = Some(6);
        car.passengers.push(1);

        for &id in segments.iter().rev().take(2) {
            let removed = tower.demolish(id).unwrap();
            traffic.sync_with_tower_after_removal(&tower, &removed);
        }

        let top = f32::from(floor_ordinal(4));
        let shaft = &traffic.elevators[0];
        assert_eq!(shaft.served_floors, vec![1, 2, 3, 4]);
        assert_eq!(shaft.cars[0].floor_position, top);
        assert!(shaft.cars[0].passengers.is_empty());
        for id in [1, 2, 3] {
            assert_eq!(
                activity_of(&traffic, id),
                PersonActivity::LeavingElevatorQueue,
                "person {id}"
            );
        }
        assert_eq!(activity_of(&traffic, 4), waiting);
        // Floor 6 went first, so the rider stepped off at what was then the top landing.
        let rider = traffic.people.iter().find(|person| person.id == 1).unwrap();
        assert_eq!(rider.current_floor, 5);
        assert_eq!(rider.floor_position, f32::from(floor_ordinal(5)));
        assert!(traffic.validate_save_state(&tower).is_ok());

        for _ in 0..200 {
            traffic.advance_after_sync(&tower, 0.1);
            assert!(
                traffic.elevators[0]
                    .cars
                    .iter()
                    .all(|car| car.floor_position <= top)
            );
        }
    }

    #[test]
    fn demolishing_a_shafts_lowest_segment_keeps_its_travellers_on_the_renumbered_shaft() {
        let (mut tower, mut traffic, segments) = shaft_with_offices(4);
        let old_id = traffic.elevators[0].id;
        assert_eq!(old_id, segments[0]);
        let rider = shaft_traveller(
            &tower,
            &traffic,
            1,
            2,
            4,
            PersonActivity::RidingElevator { shaft_id: old_id },
        );
        let waiter = shaft_traveller(
            &tower,
            &traffic,
            2,
            2,
            3,
            PersonActivity::WaitingForElevator { shaft_id: old_id },
        );
        traffic.people.extend([rider, waiter]);
        let car = &mut traffic.elevators[0].cars[0];
        car.floor_position = f32::from(floor_ordinal(2));
        car.passengers.push(1);

        let removed = tower.demolish(segments[0]).unwrap();
        traffic.sync_with_tower_after_removal(&tower, &removed);

        let new_id = traffic.elevators[0].id;
        assert_ne!(new_id, old_id);
        assert_eq!(
            activity_of(&traffic, 1),
            PersonActivity::RidingElevator { shaft_id: new_id }
        );
        assert_eq!(
            activity_of(&traffic, 2),
            PersonActivity::WaitingForElevator { shaft_id: new_id }
        );
        assert!(traffic.people.iter().all(|person| matches!(
            person.route,
            TravelMode::Elevator { shaft_id, .. } if shaft_id == new_id
        )));
        assert_eq!(traffic.elevators[0].cars[0].passengers, vec![1]);
        assert!(traffic.validate_save_state(&tower).is_ok());
    }

    #[test]
    fn demolishing_a_shafts_lowest_segment_renumbers_away_residents_way_home() {
        let (mut tower, mut traffic, segments) = shaft_with_offices(4);
        let old_id = traffic.elevators[0].id;
        let home = tower
            .facilities()
            .iter()
            .find(|facility| facility.kind == FacilityKind::Office && facility.position.floor == 3)
            .unwrap()
            .clone();
        let lobby_route = TravelMode::Elevator {
            shaft_id: old_id,
            x: shaft_center(&traffic.elevators[0]),
        };
        traffic.away.push(resident_out(1, &home, lobby_route, 30.0));

        let removed = tower.demolish(segments[0]).unwrap();
        traffic.sync_with_tower_after_removal(&tower, &removed);

        let new_id = traffic.elevators[0].id;
        assert_ne!(new_id, old_id);
        assert!(matches!(
            traffic.away[0].route,
            TravelMode::Elevator { shaft_id, .. } if shaft_id == new_id
        ));
    }

    #[test]
    fn demolishing_a_whole_shaft_leaves_nobody_riding_or_waiting_for_it() {
        let (mut tower, mut traffic, segments) = shaft_with_offices(3);
        let shaft_id = traffic.elevators[0].id;
        let rider = shaft_traveller(
            &tower,
            &traffic,
            1,
            1,
            3,
            PersonActivity::RidingElevator { shaft_id },
        );
        let waiter = shaft_traveller(
            &tower,
            &traffic,
            2,
            3,
            3,
            PersonActivity::WaitingForElevator { shaft_id },
        );
        traffic.people.extend([rider, waiter]);
        let car = &mut traffic.elevators[0].cars[0];
        car.floor_position = 1.4;
        car.passengers.push(1);

        // Bottom up, so the rider's floor stays served until the shaft is gone.
        for &id in &segments {
            let removed = tower.demolish(id).unwrap();
            traffic.sync_with_tower_after_removal(&tower, &removed);
        }

        assert!(traffic.elevators.is_empty());
        for person in &traffic.people {
            assert_eq!(person.activity, PersonActivity::LeavingElevatorQueue);
        }
        let rider = traffic.people.iter().find(|person| person.id == 1).unwrap();
        assert_eq!(
            rider.current_floor, 3,
            "the rider steps off where the car was"
        );
        assert_eq!(rider.floor_position, f32::from(floor_ordinal(3)));
        assert!(traffic.validate_save_state(&tower).is_ok());
    }

    #[test]
    fn first_elevator_car_starts_at_the_placement_floor() {
        let mut tower = Tower::new(32, 10_000_000);
        place_floor_run(&mut tower, 5, 32);
        tower
            .place(FacilityKind::Elevator, GridPosition { x: 8, floor: 5 })
            .unwrap();

        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        let car = &traffic.elevators[0].cars[0];
        assert_eq!(traffic.elevators[0].cars.len(), 1);
        assert_eq!(car.home_floor, 5);
        assert_eq!(car.floor_position, f32::from(floor_ordinal(5)));
    }

    #[test]
    fn people_visit_businesses_via_an_elevator() {
        let tower = traffic_tower(true);
        let mut traffic = TrafficSimulation::new();
        for _ in 0..240 {
            traffic.advance(&tower, 0.25);
        }
        assert!(!traffic.people.is_empty());
        assert!(traffic.people.iter().any(|person| {
            person.current_floor == 1
                || matches!(person.activity, PersonActivity::RidingElevator { .. })
        }));
    }

    #[test]
    fn a_waiting_person_progresses_from_calm_to_concerned_to_angry() {
        let mut tower = traffic_tower(true);
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        traffic.spawn_person(&tower);
        let shaft_id = traffic.elevators[0].id;
        let person = &mut traffic.people[0];
        person.activity = PersonActivity::WaitingForElevator { shaft_id };
        advance_people(&mut traffic.people, CONCERNED_WAIT_SECONDS + 0.1);
        assert_eq!(traffic.people[0].mood, PersonMood::Concerned);
        advance_people(
            &mut traffic.people,
            ANGRY_WAIT_SECONDS - CONCERNED_WAIT_SECONDS,
        );
        assert_eq!(traffic.people[0].mood, PersonMood::Angry);

        tower.clock.speed = crate::SimulationSpeed::Paused;
    }

    #[test]
    fn elevator_queues_are_capped_per_shaft_and_floor() {
        let tower = traffic_tower(true);
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        let shaft_id = traffic.elevators[0].id;
        for _ in 0..MAX_ELEVATOR_QUEUE_PER_FLOOR + 3 {
            assert!(traffic.spawn_person(&tower));
        }
        for person in &mut traffic.people {
            person.activity = PersonActivity::WaitingForElevator { shaft_id };
            person.current_floor = 1;
        }

        enforce_elevator_queue_limits(&mut traffic.people);

        assert_eq!(
            traffic
                .people
                .iter()
                .filter(|person| matches!(
                    person.activity,
                    PersonActivity::WaitingForElevator { .. }
                ))
                .count(),
            MAX_ELEVATOR_QUEUE_PER_FLOOR
        );
        assert_eq!(
            traffic
                .people
                .iter()
                .filter(|person| person.activity == PersonActivity::LeavingElevatorQueue)
                .count(),
            3
        );
    }

    #[test]
    fn angry_elevator_waiters_eventually_abandon_the_queue() {
        let tower = traffic_tower(true);
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        assert!(traffic.spawn_person(&tower));
        let shaft_id = traffic.elevators[0].id;
        let person = &mut traffic.people[0];
        person.activity = PersonActivity::WaitingForElevator { shaft_id };
        person.wait_seconds = ABANDON_ELEVATOR_WAIT_SECONDS;

        enforce_elevator_queue_limits(&mut traffic.people);

        assert_eq!(
            traffic.people[0].activity,
            PersonActivity::LeavingElevatorQueue
        );
        assert_eq!(traffic.people[0].mood, PersonMood::Angry);
        advance_people(&mut traffic.people, QUEUE_ABANDON_WALK_SECONDS + 0.1);
        assert!(traffic.people[0].visit_remaining <= 0.0);
    }

    #[test]
    fn original_default_schedule_values_are_exposed() {
        let schedule = ElevatorSchedule::default();
        assert_eq!(schedule.mode, ElevatorMode::Local);
        assert_eq!(schedule.waiting_car_response, 5);
        assert_eq!(schedule.departure_seconds, 0);
        assert_eq!(ELEVATOR_CAPACITY, 17);
        assert_eq!(EXPRESS_ELEVATOR_CAPACITY, 34);
        assert_eq!(elevator_capacity(FacilityKind::ExpressElevator), 34);
    }

    #[test]
    fn express_elevators_only_stop_at_basements_ground_and_sky_lobbies() {
        let shaft = test_shaft(1, FacilityKind::ExpressElevator, 8, &[-2, -1, 1, 2, 15, 30]);
        let facilities = [
            test_facility(1, FacilityKind::Lobby, 8, 1),
            test_facility(2, FacilityKind::Lobby, 8, 15),
        ];
        assert!(shaft_stops_at(&shaft, -2, &facilities));
        assert!(shaft_stops_at(&shaft, 1, &facilities));
        assert!(shaft_stops_at(&shaft, 15, &facilities));
        assert!(!shaft_stops_at(&shaft, 2, &facilities));
        assert!(!shaft_stops_at(&shaft, 30, &facilities));
    }

    #[test]
    fn routing_chooses_the_closest_direct_elevator() {
        let elevators = [
            test_shaft(1, FacilityKind::Elevator, 40, &[1, 2]),
            test_shaft(2, FacilityKind::Elevator, 5, &[1, 2]),
        ];
        let route = choose_route(
            Endpoint {
                facility_id: 0,
                x: 0.0,
                floor: 1,
            },
            Endpoint {
                facility_id: 9,
                x: 12.0,
                floor: 2,
            },
            &elevators,
            &[],
            &[],
        );
        assert!(matches!(
            route,
            Some(TravelMode::Elevator { shaft_id: 2, .. })
        ));
    }

    #[test]
    fn housekeeping_routes_are_exclusive_to_service_elevators() {
        let public = test_shaft(1, FacilityKind::Elevator, 4, &[1, 2]);
        let express = test_shaft(2, FacilityKind::ExpressElevator, 10, &[1, 2]);
        let service = test_shaft(3, FacilityKind::ServiceElevator, 20, &[1, 2]);
        let origin = Endpoint {
            facility_id: 10,
            x: 0.0,
            floor: 1,
        };
        let destination = Endpoint {
            facility_id: 11,
            x: 30.0,
            floor: 2,
        };

        let housekeeping_route = choose_route_scored_for(
            origin,
            destination,
            &[public.clone(), express.clone(), service.clone()],
            &[],
            &[],
            ElevatorAccess::Housekeeping,
        )
        .map(|(route, _)| route);
        assert!(matches!(
            housekeeping_route,
            Some(TravelMode::Elevator { shaft_id: 3, .. })
        ));

        assert!(
            choose_route_scored_for(
                origin,
                destination,
                &[public, express],
                &[],
                &[],
                ElevatorAccess::Housekeeping,
            )
            .is_none()
        );
        let public_route = choose_route_scored_for(
            origin,
            destination,
            &[service],
            &[],
            &[],
            ElevatorAccess::Public,
        );
        assert!(public_route.is_none());
    }

    #[test]
    fn dispatched_housekeeper_uses_the_dedicated_worker_lifecycle() {
        let mut tower = Tower::new(64, 10_000_000);
        for floor in [1, 2] {
            for x in 0..64 {
                tower.place_floor(GridPosition { x, floor }).unwrap();
            }
        }
        let housekeeping_id = tower
            .place(FacilityKind::Housekeeping, GridPosition { x: 0, floor: 1 })
            .unwrap();
        tower
            .place(
                FacilityKind::ServiceElevator,
                GridPosition { x: 20, floor: 1 },
            )
            .unwrap();
        tower
            .place(
                FacilityKind::ServiceElevator,
                GridPosition { x: 20, floor: 2 },
            )
            .unwrap();
        let room_id = tower
            .place(FacilityKind::HotelSingle, GridPosition { x: 30, floor: 2 })
            .unwrap();
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);

        let worker_id = traffic
            .dispatch_housekeeper(&tower, housekeeping_id, room_id, 8.0)
            .expect("a reachable room should receive a housekeeping worker");
        let worker = traffic
            .people()
            .iter()
            .find(|person| person.id == worker_id)
            .unwrap();
        assert_eq!(worker.home_facility_id, housekeeping_id);
        assert_eq!(worker.destination_facility_id, room_id);
        let TravelMode::Elevator { shaft_id, .. } = worker.route else {
            panic!("housekeeping should use the service elevator");
        };
        assert_eq!(
            traffic
                .elevators()
                .iter()
                .find(|shaft| shaft.id == shaft_id)
                .unwrap()
                .kind,
            FacilityKind::ServiceElevator
        );
        assert!(
            traffic
                .dispatch_housekeeper(&tower, housekeeping_id, room_id, 8.0)
                .is_none(),
            "a dirty room must not receive duplicate workers"
        );

        let worker = traffic
            .people
            .iter_mut()
            .find(|person| person.id == worker_id)
            .unwrap();
        worker.route = TravelMode::SameFloor;
        worker.x = worker.destination.x;
        worker.current_floor = worker.destination.floor;
        worker.floor_position = f32::from(floor_ordinal(worker.destination.floor));
        worker.walking_from_transport = true;
        worker.activity = PersonActivity::Walking;
        traffic.advance_after_sync(&tower, 0.1);
        assert_eq!(traffic.people[0].activity, PersonActivity::Visiting);
        assert_eq!(traffic.visitors_at(room_id), 0);
        assert_eq!(traffic.daily_visitors_at(room_id), 0);
        assert!(!traffic.has_received_visitor(room_id));
    }

    #[test]
    fn routing_allows_one_change_at_a_lobby() {
        let elevators = [
            test_shaft(1, FacilityKind::ExpressElevator, 8, &[1, 15]),
            test_shaft(2, FacilityKind::Elevator, 24, &[15, 16, 17]),
        ];
        let mut lobbies = vec![test_facility(3, FacilityKind::Lobby, 8, 1)];
        lobbies.extend(
            (12..=24).map(|x| test_facility(100 + u64::from(x), FacilityKind::Lobby, x, 15)),
        );
        let route = choose_route(
            Endpoint {
                facility_id: 0,
                x: -12.0,
                floor: 1,
            },
            Endpoint {
                facility_id: 4,
                x: 40.0,
                floor: 17,
            },
            &elevators,
            &lobbies,
            &[],
        );
        assert!(matches!(
            route,
            Some(TravelMode::ElevatorTransfer {
                first_id: 1,
                transfer_floor: 15,
                second_id: 2,
                ..
            })
        ));
    }

    #[test]
    fn disconnected_sky_lobby_does_not_enable_an_elevator_transfer() {
        let elevators = [
            test_shaft(1, FacilityKind::ExpressElevator, 8, &[1, 15]),
            test_shaft(2, FacilityKind::Elevator, 24, &[15, 16, 17]),
        ];
        let lobbies = [
            test_facility(3, FacilityKind::Lobby, 8, 1),
            test_facility(4, FacilityKind::Lobby, 8, 15),
            test_facility(5, FacilityKind::Lobby, 24, 15),
        ];
        let route = choose_route(
            Endpoint {
                facility_id: 3,
                x: 8.5,
                floor: 1,
            },
            Endpoint {
                facility_id: 9,
                x: 40.0,
                floor: 17,
            },
            &elevators,
            &lobbies,
            &[],
        );
        assert!(route.is_none());
    }

    #[test]
    fn express_elevator_needs_a_real_lobby_at_each_above_ground_stop() {
        let elevators = [test_shaft(1, FacilityKind::ExpressElevator, 8, &[1, 15])];
        let ground_lobby = test_facility(3, FacilityKind::Lobby, 8, 1);
        let route = choose_route(
            endpoint(&ground_lobby),
            Endpoint {
                facility_id: 9,
                x: 20.0,
                floor: 15,
            },
            &elevators,
            &[ground_lobby],
            &[],
        );
        assert!(route.is_none());
    }

    /// Floors 1-15 with lobbies on 1 and 15, an express from 1 to 15, and an office on 14
    /// whose only way in is the express to the sky lobby and a staircase down from it.
    fn office_below_a_sky_lobby(stairs_x: u16) -> (Tower, u64, u64) {
        let mut tower = Tower::new(64, 1_000_000_000);
        place_floor_run(&mut tower, 15, 64);
        for floor in [1, 15] {
            for x in 0..20 {
                tower
                    .place(FacilityKind::Lobby, GridPosition { x, floor })
                    .unwrap();
            }
        }
        for floor in 1..=15 {
            tower
                .place(FacilityKind::ExpressElevator, GridPosition { x: 20, floor })
                .unwrap();
        }
        let stairs = tower
            .place(
                FacilityKind::Stairs,
                GridPosition {
                    x: stairs_x,
                    floor: 14,
                },
            )
            .unwrap();
        let office = tower
            .place(FacilityKind::Office, GridPosition { x: 40, floor: 14 })
            .unwrap();
        tower.advance_time(60.0);
        (tower, office, stairs)
    }

    #[test]
    fn a_floor_beside_a_sky_lobby_is_reached_by_its_express_and_the_stairs() {
        let (tower, office, _) = office_below_a_sky_lobby(4);
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);

        let route = *traffic.routes_from_lobby.get(&office).unwrap();
        let TravelMode::ElevatorThenWalkway {
            shaft_id,
            transfer_floor,
            ..
        } = route
        else {
            panic!("expected an express ride and a stair climb, got {route:?}");
        };
        assert_eq!(shaft_id, traffic.elevators[0].id);
        assert_eq!(transfer_floor, 15);
        assert_eq!(reverse_route(reverse_route(route)), route);
    }

    #[test]
    fn a_direct_elevator_keeps_priority_over_a_sky_lobby_stair_route() {
        let mut tower = Tower::new(128, 1_000_000_000);
        place_floor_run(&mut tower, 15, 128);
        for (floor, columns) in [(1, 0..20), (1, 104..112), (15, 0..20)] {
            for x in columns {
                tower
                    .place(FacilityKind::Lobby, GridPosition { x, floor })
                    .unwrap();
            }
        }
        for floor in 1..=15 {
            tower
                .place(FacilityKind::ExpressElevator, GridPosition { x: 20, floor })
                .unwrap();
        }
        // A local far across the tower, whose ride costs more walking than the express and
        // the stairs beside the office.
        for floor in 1..=14 {
            tower
                .place(FacilityKind::Elevator, GridPosition { x: 100, floor })
                .unwrap();
        }
        tower
            .place(FacilityKind::Stairs, GridPosition { x: 4, floor: 14 })
            .unwrap();
        let office = tower
            .place(FacilityKind::Office, GridPosition { x: 10, floor: 14 })
            .unwrap();
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);

        let local = traffic
            .elevators
            .iter()
            .find(|shaft| shaft.kind == FacilityKind::Elevator)
            .unwrap()
            .id;
        assert!(matches!(
            traffic.routes_from_lobby.get(&office),
            Some(TravelMode::Elevator { shaft_id, .. }) if *shaft_id == local
        ));
    }

    #[test]
    fn a_stair_the_sky_lobby_does_not_reach_is_no_transfer() {
        let (tower, office, _) = office_below_a_sky_lobby(40);
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        assert!(!traffic.routes_from_lobby.contains_key(&office));
    }

    #[test]
    fn people_ride_to_the_sky_lobby_then_climb_down_and_back() {
        let (tower, office, _) = office_below_a_sky_lobby(4);
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        for _ in 0..800 {
            traffic.advance(&tower, 0.25);
        }
        assert!(traffic.visitors_at(office) > 0);

        // Send one worker home and follow them out of the tower.
        let worker = traffic
            .people
            .iter_mut()
            .find(|person| {
                person.destination_facility_id == office
                    && person.activity == PersonActivity::Visiting
            })
            .unwrap();
        let worker_id = worker.id;
        worker.visit_remaining = 0.1;
        let shaft_id = traffic.elevators[0].id;
        let mut climbed_up = false;
        let mut waited_at_sky_lobby = false;
        let mut rode_down = false;
        let mut gave_up = false;
        for _ in 0..2_000 {
            traffic.advance(&tower, 0.25);
            let Some(worker) = traffic.people.iter().find(|person| person.id == worker_id) else {
                break;
            };
            climbed_up |= worker.activity == PersonActivity::UsingStairs;
            waited_at_sky_lobby |= worker.activity
                == PersonActivity::WaitingForElevator { shaft_id }
                && worker.current_floor == 15;
            rode_down |= worker.activity == PersonActivity::RidingElevator { shaft_id };
            gave_up |= worker.activity == PersonActivity::LeavingElevatorQueue;
        }
        assert!(climbed_up && waited_at_sky_lobby && rode_down);
        assert!(!gave_up);
        assert!(traffic.people.iter().all(|person| person.id != worker_id));
    }

    #[test]
    fn demolishing_the_stairs_cuts_the_sky_lobby_route() {
        let (mut tower, office, stairs) = office_below_a_sky_lobby(4);
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        assert!(traffic.routes_from_lobby.contains_key(&office));

        let removed = tower.demolish(stairs).unwrap();
        traffic.sync_with_tower_after_removal(&tower, &removed);

        assert!(!traffic.routes_from_lobby.contains_key(&office));
    }

    #[test]
    fn elevators_skip_the_open_middle_of_a_grand_lobby() {
        let shaft = test_shaft(1, FacilityKind::Elevator, 8, &[1, 2, 3, 4]);
        let mut lobby = test_facility(2, FacilityKind::Lobby, 12, 1);
        lobby.stories = 3;
        assert!(shaft_stops_at(&shaft, 1, &[lobby.clone()]));
        assert!(!shaft_stops_at(&shaft, 2, &[lobby.clone()]));
        assert!(shaft_stops_at(&shaft, 3, &[lobby.clone()]));
        assert!(shaft_stops_at(&shaft, 4, &[lobby.clone()]));

        let mut saved_shaft = shaft;
        saved_shaft.cars.push(ElevatorCar {
            floor_position: f32::from(floor_ordinal(2)),
            home_floor: 2,
            direction: ElevatorDirection::Idle,
            target_floor: None,
            passengers: Vec::new(),
            dwell_remaining: 0.0,
        });
        normalize_elevator_home_floors(std::slice::from_mut(&mut saved_shaft), &[lobby]);
        assert_eq!(saved_shaft.cars[0].home_floor, 1);
    }

    #[test]
    fn one_walkway_expands_through_a_three_story_lobby() {
        let walkway = test_facility(1, FacilityKind::Escalator, 8, 1);
        let mut lobby = test_facility(2, FacilityKind::Lobby, 8, 1);
        lobby.stories = 3;
        let facilities = [walkway.clone(), lobby];
        assert_eq!(vertical_walkway_story_span(&walkway, &facilities), 3);
        assert!(
            connected_vertical_walkways(1, 3, &facilities)
                .iter()
                .any(|(kind, _)| *kind == FacilityKind::Escalator)
        );
        assert!(connected_vertical_walkways(1, 2, &facilities).is_empty());
    }

    #[test]
    fn stairs_cover_four_flights_and_escalators_cover_three() {
        let stairs = (1..=5)
            .map(|floor| test_facility(floor as u64, FacilityKind::Stairs, 8, floor))
            .collect::<Vec<_>>();
        assert!(matches!(
            choose_route(
                Endpoint {
                    facility_id: 0,
                    x: 0.0,
                    floor: 1,
                },
                Endpoint {
                    facility_id: 8,
                    x: 20.0,
                    floor: 5,
                },
                &[],
                &stairs,
                &[],
            ),
            Some(TravelMode::Stairs { .. })
        ));
        assert!(
            choose_route(
                Endpoint {
                    facility_id: 0,
                    x: 0.0,
                    floor: 1,
                },
                Endpoint {
                    facility_id: 8,
                    x: 20.0,
                    floor: 6,
                },
                &[],
                &stairs,
                &[],
            )
            .is_none()
        );

        let escalators = (1..=4)
            .map(|floor| test_facility(floor as u64, FacilityKind::Escalator, 16, floor))
            .collect::<Vec<_>>();
        assert!(matches!(
            choose_route(
                Endpoint {
                    facility_id: 0,
                    x: 0.0,
                    floor: 1,
                },
                Endpoint {
                    facility_id: 10,
                    x: 20.0,
                    floor: 4,
                },
                &[],
                &escalators,
                &[],
            ),
            Some(TravelMode::Stairs { .. })
        ));
        assert!(
            choose_route(
                Endpoint {
                    facility_id: 0,
                    x: 0.0,
                    floor: 1,
                },
                Endpoint {
                    facility_id: 10,
                    x: 20.0,
                    floor: 5,
                },
                &[],
                &escalators,
                &[],
            )
            .is_none()
        );
    }

    #[test]
    fn people_prefer_an_elevator_as_the_stair_climb_gets_longer() {
        let stairs = [
            test_facility(1, FacilityKind::Stairs, 8, 1),
            test_facility(2, FacilityKind::Stairs, 8, 2),
            test_facility(3, FacilityKind::Stairs, 8, 3),
        ];
        let elevator = [test_shaft(9, FacilityKind::Elevator, 8, &[1, 2, 3, 4])];
        let route = choose_route(
            Endpoint {
                facility_id: 0,
                x: 0.0,
                floor: 1,
            },
            Endpoint {
                facility_id: 10,
                x: 20.0,
                floor: 4,
            },
            &elevator,
            &stairs,
            &[],
        );
        assert!(matches!(
            route,
            Some(TravelMode::Elevator { shaft_id: 9, .. })
        ));
    }

    #[test]
    fn tenant_reachability_requires_a_first_floor_lobby_and_complete_path() {
        let mut tower = Tower::new(64, 10_000_000);
        place_floor_run(&mut tower, 2, 64);
        let office_id = tower
            .place(FacilityKind::Office, GridPosition { x: 40, floor: 2 })
            .unwrap();
        tower
            .place(FacilityKind::Elevator, GridPosition { x: 32, floor: 1 })
            .unwrap();
        tower
            .place(FacilityKind::Elevator, GridPosition { x: 32, floor: 2 })
            .unwrap();
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        assert!(!traffic.reachable_facility_ids(&tower).contains(&office_id));

        for x in 24..=32 {
            tower
                .place(FacilityKind::Lobby, GridPosition { x, floor: 1 })
                .unwrap();
        }
        traffic.sync_with_tower(&tower);
        assert!(traffic.reachable_facility_ids(&tower).contains(&office_id));

        let mut split_tower = Tower::new(64, 10_000_000);
        for x in 0..16 {
            split_tower
                .place_floor(GridPosition { x, floor: 1 })
                .unwrap();
        }
        for x in 32..64 {
            split_tower
                .place_floor(GridPosition { x, floor: 1 })
                .unwrap();
            split_tower
                .place_floor(GridPosition { x, floor: 2 })
                .unwrap();
        }
        split_tower
            .place(FacilityKind::Lobby, GridPosition { x: 4, floor: 1 })
            .unwrap();
        let split_office = split_tower
            .place(FacilityKind::Office, GridPosition { x: 40, floor: 2 })
            .unwrap();
        split_tower
            .place(FacilityKind::Elevator, GridPosition { x: 32, floor: 1 })
            .unwrap();
        split_tower
            .place(FacilityKind::Elevator, GridPosition { x: 32, floor: 2 })
            .unwrap();
        traffic.sync_with_tower(&split_tower);
        assert!(
            !traffic
                .reachable_facility_ids(&split_tower)
                .contains(&split_office)
        );
    }

    #[test]
    fn businesses_only_generate_arrivals_during_open_hours() {
        assert!(!facility_is_open(FacilityKind::FastFood, 9 * 60 + 59));
        assert!(facility_is_open(FacilityKind::FastFood, 10 * 60));
        assert!(facility_is_open(FacilityKind::FastFood, 20 * 60 + 59));
        assert!(!facility_is_open(FacilityKind::FastFood, 21 * 60));
        assert!(!facility_is_open(FacilityKind::Office, 8 * 60 + 59));
        assert!(facility_is_open(FacilityKind::Office, 9 * 60));
        assert!(!facility_is_open(FacilityKind::Office, 17 * 60));
        assert!(!facility_is_open(FacilityKind::Restaurant, 16 * 60 + 59));
        assert!(facility_is_open(FacilityKind::Restaurant, 17 * 60));
        assert!(facility_is_open(FacilityKind::Restaurant, 22 * 60));
        assert!(!facility_is_open(FacilityKind::Restaurant, 23 * 60));
    }

    #[test]
    fn a_shaft_can_have_multiple_independent_cars() {
        let tower = traffic_tower(true);
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        let shaft_id = traffic.elevators[0].id;
        assert!(traffic.add_elevator_car(&tower, shaft_id, 2));
        assert_eq!(traffic.elevators[0].cars.len(), 2);
        assert_eq!(traffic.elevators[0].cars[0].home_floor, 1);
        assert_eq!(traffic.elevators[0].cars[1].home_floor, 2);
        assert!(!traffic.add_elevator_car(&tower, shaft_id, 3));
    }

    #[test]
    fn a_car_keeps_its_assigned_hall_call_while_approaching() {
        let tower = traffic_tower(true);
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        let shaft_id = traffic.elevators[0].id;
        assert!(traffic.add_elevator_car(&tower, shaft_id, 2));
        assert!(traffic.spawn_person(&tower));

        let person = &mut traffic.people[0];
        person.current_floor = 1;
        person.floor_position = f32::from(floor_ordinal(1));
        person.destination.floor = 2;
        person.route = TravelMode::Elevator {
            shaft_id,
            x: f32::from(traffic.elevators[0].x),
        };
        person.activity = PersonActivity::WaitingForElevator { shaft_id };

        let cars = &mut traffic.elevators[0].cars;
        cars[0].direction = ElevatorDirection::Up;
        cars[0].target_floor = Some(2);
        cars[1].direction = ElevatorDirection::Down;
        cars[1].target_floor = Some(1);

        let assignments = assign_waiting_calls(&traffic.elevators[0], &traffic.people, 5);
        assert_eq!(assignments, vec![(1, 1)]);
    }

    /// Waiting on floor 2 to go down, with a full car stopped on floor 2 heading down and an
    /// empty idle car below. The full car used to win the call and re-stop on floor 2 forever.
    fn full_car_at_caller_floor(add_idle_car: bool) -> (Tower, TrafficSimulation) {
        let tower = traffic_tower(true);
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        let shaft_id = traffic.elevators[0].id;
        if add_idle_car {
            assert!(traffic.add_elevator_car(&tower, shaft_id, 2));
        }
        assert!(traffic.spawn_person(&tower));

        let person = &mut traffic.people[0];
        person.current_floor = 2;
        person.floor_position = f32::from(floor_ordinal(2));
        person.destination.floor = 1;
        person.route = TravelMode::Elevator {
            shaft_id,
            x: f32::from(traffic.elevators[0].x),
        };
        person.activity = PersonActivity::WaitingForElevator { shaft_id };

        let capacity = elevator_capacity(traffic.elevators[0].kind) as u64;
        let cars = &mut traffic.elevators[0].cars;
        cars[0].floor_position = f32::from(floor_ordinal(2));
        cars[0].direction = ElevatorDirection::Down;
        cars[0].target_floor = None;
        cars[0].passengers = (10_000..10_000 + capacity).collect();
        if add_idle_car {
            // Park the idle car at its own home on floor 1, so only a dispatched hall call
            // (not a trip home) can bring it to the caller's floor.
            cars[1].home_floor = 1;
            cars[1].floor_position = f32::from(floor_ordinal(1));
            cars[1].direction = ElevatorDirection::Idle;
            cars[1].target_floor = None;
        }
        (tower, traffic)
    }

    #[test]
    fn a_full_car_never_takes_a_hall_call_from_a_car_with_room() {
        let (_, traffic) = full_car_at_caller_floor(true);
        let assignments = assign_waiting_calls(&traffic.elevators[0], &traffic.people, 5);
        assert_eq!(assignments, vec![(2, 1)]);
    }

    #[test]
    fn a_hall_call_stays_unassigned_while_every_car_is_full() {
        let (_, traffic) = full_car_at_caller_floor(false);
        let assignments = assign_waiting_calls(&traffic.elevators[0], &traffic.people, 5);
        assert!(assignments.is_empty());
    }

    #[test]
    fn a_full_car_departs_and_an_idle_car_collects_the_waiting_person() {
        let (tower, mut traffic) = full_car_at_caller_floor(true);
        let shaft_id = traffic.elevators[0].id;
        for _ in 0..40 {
            advance_shaft(
                &mut traffic.elevators[0],
                &mut traffic.people,
                tower.clock,
                0.25,
            );
        }
        assert!(traffic.elevators[0].cars[0].floor_position < f32::from(floor_ordinal(2)) - 0.5);
        // Picked up by the idle car; within ten seconds it may already have been delivered.
        assert!(!matches!(
            traffic.people[0].activity,
            PersonActivity::WaitingForElevator { shaft_id: waiting } if waiting == shaft_id
        ));
    }

    /// A shaft over floors 1-30 with one waiter on `floor` bound upward, and cars at the given
    /// (ordinal position, target, direction).
    fn hall_call(floor: i16, cars: &[(f32, Option<i16>, ElevatorDirection)]) -> Vec<(i16, usize)> {
        let mut shaft = test_shaft(1, FacilityKind::Elevator, 8, &(1..=30).collect::<Vec<_>>());
        shaft.cars = cars
            .iter()
            .map(|&(floor_position, target_floor, direction)| ElevatorCar {
                floor_position,
                home_floor: 1,
                direction,
                target_floor,
                passengers: Vec::new(),
                dwell_remaining: 0.0,
            })
            .collect();
        let waiter = Person {
            id: 7,
            role: PersonRole::Visitor,
            home_facility_id: 0,
            destination_facility_id: 0,
            x: 10.0,
            floor_position: f32::from(floor_ordinal(floor)),
            current_floor: floor,
            mood: PersonMood::Calm,
            activity: PersonActivity::WaitingForElevator { shaft_id: 1 },
            wait_seconds: 0.0,
            origin: Endpoint {
                facility_id: 0,
                x: 0.0,
                floor,
            },
            destination: Endpoint {
                facility_id: 0,
                x: 10.0,
                floor: 25,
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
        assign_waiting_calls(&shaft, &[waiter], 5)
    }

    #[test]
    fn an_idle_car_at_the_callers_floor_takes_the_call_from_a_car_heading_home() {
        // Car 1 is returning to its home floor, the lobby, from floor 25.
        let assignments = hall_call(
            1,
            &[
                (0.0, None, ElevatorDirection::Idle),
                (24.0, Some(1), ElevatorDirection::Down),
            ],
        );
        assert_eq!(assignments, vec![(1, 0)]);
    }

    #[test]
    fn a_far_car_heading_to_the_callers_floor_yields_to_a_near_idle_car() {
        let assignments = hall_call(
            10,
            &[
                (8.0, None, ElevatorDirection::Idle),
                (28.0, Some(10), ElevatorDirection::Down),
            ],
        );
        assert_eq!(assignments, vec![(10, 0)]);
    }

    #[test]
    fn a_car_heading_to_the_callers_floor_keeps_it_within_the_dispatch_threshold() {
        // Car 1 is three floors away against the idle car's one: within the threshold of 5.
        let assignments = hall_call(
            10,
            &[
                (8.0, None, ElevatorDirection::Idle),
                (12.0, Some(10), ElevatorDirection::Down),
            ],
        );
        assert_eq!(assignments, vec![(10, 1)]);
    }

    #[test]
    fn an_elevator_shaft_accepts_at_most_eight_cars() {
        let mut tower = Tower::new(32, 10_000_000);
        place_floor_run(&mut tower, 8, 32);
        for floor in 1..=8 {
            tower
                .place(FacilityKind::Elevator, GridPosition { x: 8, floor })
                .unwrap();
        }
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        let shaft_id = traffic.elevators[0].id;
        for floor in 2..=8 {
            assert!(traffic.add_elevator_car(&tower, shaft_id, floor));
        }
        assert_eq!(traffic.elevators[0].cars.len(), MAX_ELEVATOR_CARS);
        assert!(!traffic.add_elevator_car(&tower, shaft_id, 8));
        assert_eq!(traffic.elevators[0].cars.len(), MAX_ELEVATOR_CARS);
    }

    #[test]
    fn express_schedules_deliver_every_rider_farthest_floor_first() {
        for (mode, start, wanted, first) in [
            (ElevatorMode::ExpressToTop, 1, [4, 8], 8),
            (ElevatorMode::ExpressToBottom, 10, [6, 2], 2),
        ] {
            let mut shaft = test_shaft(1, FacilityKind::Elevator, 8, &(1..=10).collect::<Vec<_>>());
            shaft.schedules = [ElevatorSchedule {
                mode,
                ..ElevatorSchedule::default()
            }; 12];
            shaft.cars.push(ElevatorCar {
                floor_position: f32::from(floor_ordinal(start)),
                home_floor: 1,
                direction: ElevatorDirection::Idle,
                target_floor: None,
                passengers: vec![1, 2],
                dwell_remaining: 0.0,
            });
            let rider = |id: u64, floor: i16| Person {
                id,
                role: PersonRole::Visitor,
                home_facility_id: 0,
                destination_facility_id: 0,
                x: 10.0,
                floor_position: f32::from(floor_ordinal(start)),
                current_floor: start,
                mood: PersonMood::Calm,
                activity: PersonActivity::RidingElevator { shaft_id: 1 },
                wait_seconds: 0.0,
                origin: Endpoint {
                    facility_id: 0,
                    x: 0.0,
                    floor: start,
                },
                destination: Endpoint {
                    facility_id: 0,
                    x: 10.0,
                    floor,
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
            let mut people = vec![rider(1, wanted[0]), rider(2, wanted[1])];
            let mut arrivals = Vec::new();
            for _ in 0..600 {
                advance_shaft(&mut shaft, &mut people, Clock::default(), 0.1);
                for person in &people {
                    if person.activity == PersonActivity::Walking
                        && !arrivals.contains(&person.current_floor)
                    {
                        arrivals.push(person.current_floor);
                    }
                }
            }
            assert_eq!(arrivals.len(), 2, "{mode:?}: arrivals {arrivals:?}");
            assert_eq!(arrivals[0], first, "{mode:?}: arrivals {arrivals:?}");
        }
    }

    #[test]
    fn extending_an_existing_shaft_does_not_create_another_car() {
        let mut tower = Tower::new(32, 10_000_000);
        place_floor_run(&mut tower, 2, 32);
        tower
            .place(FacilityKind::Elevator, GridPosition { x: 8, floor: 1 })
            .unwrap();
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        assert_eq!(traffic.elevators[0].cars.len(), 1);

        tower
            .place(FacilityKind::Elevator, GridPosition { x: 8, floor: 2 })
            .unwrap();
        traffic.sync_with_tower(&tower);
        assert_eq!(traffic.elevators[0].served_floors, vec![1, 2]);
        assert_eq!(traffic.elevators[0].cars.len(), 1);
    }

    #[test]
    fn every_person_enters_from_outside_and_eventually_leaves() {
        let mut tower = Tower::new(64, 10_000_000);
        place_floor_run(&mut tower, 1, 64);
        tower
            .place(FacilityKind::FastFood, GridPosition { x: 24, floor: 1 })
            .unwrap();
        tower
            .place(FacilityKind::Lobby, GridPosition { x: 20, floor: 1 })
            .unwrap();
        tower.advance_time(60.0);
        let mut traffic = TrafficSimulation::new();
        for _ in 0..32 {
            traffic.advance(&tower, 0.25);
        }
        assert!(!traffic.people.is_empty());
        assert!(
            traffic
                .people
                .iter()
                .all(|person| person.home_facility_id == 0)
        );

        for _ in 0..1_600 {
            traffic.advance(&tower, 0.25);
        }
        assert!(traffic.people.len() <= usize::from(tower.star_rating()) * 32);
    }

    #[test]
    fn occupied_offices_receive_people_from_outside() {
        let mut tower = Tower::new(32, 10_000_000);
        place_floor_run(&mut tower, 1, 32);
        let office_id = tower
            .place(FacilityKind::Office, GridPosition { x: 12, floor: 1 })
            .unwrap();
        tower
            .place(FacilityKind::Lobby, GridPosition { x: 4, floor: 1 })
            .unwrap();
        tower.advance_time(30.0);

        let mut traffic = TrafficSimulation::new();
        for _ in 0..800 {
            traffic.advance(&tower, 0.25);
        }

        assert!(traffic.visitors_at(office_id) > 0);
        assert!(traffic.has_received_visitor(office_id));
        assert!(
            traffic
                .people()
                .iter()
                .all(|person| person.home_facility_id == 0)
        );
    }

    /// A resident of `home` standing at the street partway through an outside trip, the state
    /// a school or work trip reaches once the resident has walked out.
    fn resident_out(id: u64, home: &Facility, lobby_route: TravelMode, remaining: f32) -> Person {
        let street = Endpoint {
            facility_id: 0,
            x: 0.0,
            floor: 1,
        };
        Person {
            id,
            role: PersonRole::Salesman,
            home_facility_id: home.id,
            destination_facility_id: 0,
            x: street.x,
            floor_position: 0.0,
            current_floor: 1,
            mood: PersonMood::Calm,
            activity: PersonActivity::Visiting,
            wait_seconds: 0.0,
            origin: endpoint(home),
            destination: street,
            route: reverse_route(lobby_route),
            going_home: false,
            walking_from_transport: false,
            visit_remaining: remaining,
            visit_duration: remaining,
            stair_progress: 0.0,
            route_leg: 0,
        }
    }

    fn office_and_lobby_route(
        tower: &Tower,
        traffic: &TrafficSimulation,
    ) -> (Facility, TravelMode) {
        let office = tower
            .facilities()
            .iter()
            .find(|facility| facility.kind == FacilityKind::Office)
            .unwrap()
            .clone();
        let route = *traffic.routes_from_lobby.get(&office.id).unwrap();
        (office, route)
    }

    #[test]
    fn residents_out_for_the_day_leave_room_for_office_workers() {
        let tower = traffic_tower(true);
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        let (office, route) = office_and_lobby_route(&tower, &traffic);
        // A full cap's worth of residents at work or school for the whole test.
        for id in 1..=MAX_SIMULATED_PEOPLE as u64 {
            traffic
                .people
                .push(resident_out(id, &office, route, 10_000.0));
        }
        traffic.next_person_id = MAX_SIMULATED_PEOPLE as u64 + 1;

        for _ in 0..800 {
            traffic.advance(&tower, 0.25);
        }

        assert_eq!(traffic.away.len(), MAX_SIMULATED_PEOPLE);
        assert!(traffic.visitors_at(office.id) > 0);
        assert_eq!(traffic.validate_save_state(&tower), Ok(()));
    }

    #[test]
    fn residents_wait_outside_until_the_tower_has_room_for_them() {
        let tower = traffic_tower(true);
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        let (office, route) = office_and_lobby_route(&tower, &traffic);
        traffic.away.push(resident_out(1, &office, route, 0.5));
        for id in 2..=MAX_SIMULATED_PEOPLE as u64 + 1 {
            let mut visitor = resident_out(id, &office, route, 10_000.0);
            visitor.home_facility_id = 0;
            visitor.destination_facility_id = office.id;
            traffic.people.push(visitor);
        }
        traffic.next_person_id = MAX_SIMULATED_PEOPLE as u64 + 2;

        for _ in 0..8 {
            traffic.advance(&tower, 0.25);
        }
        assert_eq!(traffic.away.len(), 1);
        assert_eq!(traffic.people.len(), MAX_SIMULATED_PEOPLE);

        traffic.people.pop();
        traffic.advance(&tower, 0.25);
        traffic.advance(&tower, 0.25);
        assert!(traffic.away.is_empty());
        let resident = traffic.people.iter().find(|person| person.id == 1).unwrap();
        assert!(resident.is_returning_home());
        assert_eq!(resident.activity, PersonActivity::Walking);
    }

    #[test]
    fn saves_keep_away_residents_and_reject_impossible_ones() {
        let tower = traffic_tower(true);
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        let (office, route) = office_and_lobby_route(&tower, &traffic);
        traffic.away.push(resident_out(1, &office, route, 30.0));
        traffic.next_person_id = 2;
        assert_eq!(traffic.validate_save_state(&tower), Ok(()));

        traffic.away[0].activity = PersonActivity::Walking;
        assert!(traffic.validate_save_state(&tower).is_err());
    }

    #[test]
    fn demolishing_an_away_residents_home_drops_them() {
        let mut tower = traffic_tower(true);
        let mut traffic = TrafficSimulation::new();
        traffic.sync_with_tower(&tower);
        let (office, route) = office_and_lobby_route(&tower, &traffic);
        traffic.away.push(resident_out(1, &office, route, 30.0));

        let removed = tower.demolish(office.id).unwrap();
        traffic.sync_with_tower_after_removal(&tower, &removed);

        assert!(traffic.away.is_empty());
    }

    #[test]
    fn weekday_office_arrivals_are_balanced_across_reachable_tenants() {
        let mut tower = Tower::new(160, 100_000_000);
        place_floor_run(&mut tower, 1, 160);
        tower
            .place(FacilityKind::Lobby, GridPosition { x: 4, floor: 1 })
            .unwrap();
        let office_ids: Vec<_> = (0..12)
            .map(|index| {
                tower
                    .place(
                        FacilityKind::Office,
                        GridPosition {
                            x: 16 + index * 10,
                            floor: 1,
                        },
                    )
                    .unwrap()
            })
            .collect();
        tower.advance_time(30.0);
        tower.clock.day = 2;
        tower.clock.minute_of_day = 12 * 60;

        let mut traffic = TrafficSimulation::new();
        for _ in 0..480 {
            traffic.advance(&tower, 0.25);
        }

        for office_id in office_ids {
            assert!(
                traffic.visitors_at(office_id) > 0,
                "reachable occupied office {office_id} should have weekday workers by noon"
            );
        }
        assert!(
            traffic.people().iter().all(|person| {
                person.origin.x < 4.0 || person.origin.x > 16.0 + 11.0 * 10.0 + 9.0
            })
        );
    }

    #[test]
    fn higher_ratings_generate_shorter_but_jittered_arrival_intervals() {
        let low = next_spawn_interval(1, 10);
        let high = next_spawn_interval(5, 10);
        assert!(high < low);
        assert_ne!(next_spawn_interval(3, 10), next_spawn_interval(3, 11));
    }

    #[test]
    fn condo_family_roles_follow_the_original_weekday_and_weekend_routine() {
        let home = test_facility(1, FacilityKind::Condo, 8, 2);
        let weekday_morning = Clock {
            day: 1,
            minute_of_day: 8 * 60 + 30,
            ..Clock::default()
        };
        let morning = scheduled_trip_specs(weekday_morning, &home, 1);
        assert!(morning.iter().any(|trip| {
            trip.role == PersonRole::Child && trip.purpose == ScheduledTripPurpose::Outside
        }));
        assert!(morning.iter().any(|trip| {
            trip.role == PersonRole::Salesman && trip.purpose == ScheduledTripPurpose::Outside
        }));

        let shopping = scheduled_trip_specs(
            Clock {
                day: 1,
                minute_of_day: 10 * 60 + 30,
                ..Clock::default()
            },
            &home,
            1,
        );
        assert_eq!(
            shopping,
            vec![ScheduledTripSpec {
                event: ScheduledTripEvent::ResidentShopping,
                role: PersonRole::Woman,
                purpose: ScheduledTripPurpose::Shopping,
                visit_seconds: 28.0,
            }]
        );

        let weekend = scheduled_trip_specs(
            Clock {
                day: 6,
                minute_of_day: 12 * 60,
                ..Clock::default()
            },
            &home,
            1,
        );
        assert_eq!(
            weekend.iter().map(|trip| trip.role).collect::<Vec<_>>(),
            [
                PersonRole::WomanWithChild,
                PersonRole::Child,
                PersonRole::Homebody,
            ]
        );
        assert!(
            weekend
                .iter()
                .all(|trip| trip.purpose == ScheduledTripPurpose::Meal)
        );
    }

    #[test]
    fn compact_schedule_stamps_keep_each_event_independent() {
        let home = 42;
        assert_ne!(
            scheduled_trip_stamp_key(home, ScheduledTripEvent::WeekendMotherMeal),
            scheduled_trip_stamp_key(home, ScheduledTripEvent::WeekendChildMeal)
        );
        assert_eq!(
            scheduled_trip_stamp_key(home, ScheduledTripEvent::WeekendChildMeal) >> 5,
            home
        );
    }

    #[test]
    fn offices_close_on_weekends_and_commercial_demand_matures() {
        let saturday = Clock {
            day: 6,
            minute_of_day: 10 * 60,
            ..Clock::default()
        };
        assert!(!facility_is_open_at(FacilityKind::Office, saturday));

        let mut traffic = TrafficSimulation::new();
        traffic.commercial_patronage.insert(7, 35);
        assert_eq!(
            traffic.dedicated_patronage_at(7, FacilityKind::FastFood, false),
            35
        );
        assert_eq!(
            traffic.dedicated_patronage_at(7, FacilityKind::FastFood, true),
            48
        );
        traffic.commercial_patronage.insert(8, 25);
        assert_eq!(
            traffic.dedicated_patronage_at(8, FacilityKind::Shop, true),
            30
        );
    }

    #[test]
    fn offices_hotels_and_cinemas_generate_their_secondary_trips() {
        let office = test_facility(1, FacilityKind::Office, 8, 4);
        let sales = scheduled_trip_specs(
            Clock {
                day: 1,
                minute_of_day: 10 * 60 + 30,
                ..Clock::default()
            },
            &office,
            3,
        );
        assert!(sales.iter().any(|trip| {
            trip.event == ScheduledTripEvent::OfficeSalesCall
                && trip.purpose == ScheduledTripPurpose::OfficeCall
        }));

        let hotel = test_facility(2, FacilityKind::HotelSuite, 16, 10);
        assert!(
            scheduled_trip_specs(
                Clock {
                    day: 6,
                    minute_of_day: 19 * 60,
                    ..Clock::default()
                },
                &hotel,
                3,
            )
            .iter()
            .any(|trip| trip.event == ScheduledTripEvent::HotelDinner)
        );

        let cinema = test_facility(3, FacilityKind::Cinema, 24, 8);
        assert!(
            scheduled_trip_specs(
                Clock {
                    day: 6,
                    minute_of_day: 20 * 60,
                    ..Clock::default()
                },
                &cinema,
                3,
            )
            .iter()
            .any(|trip| trip.event == ScheduledTripEvent::CinemaEveningDispersal)
        );
    }

    #[test]
    fn a_new_cinema_program_restores_opening_demand() {
        let mut traffic = TrafficSimulation::new();
        traffic.commercial_patronage.insert(9, 20);
        assert!(traffic.change_cinema_program(9));
        assert_eq!(
            traffic.dedicated_patronage_at(9, FacilityKind::Cinema, false),
            120
        );
    }
}
