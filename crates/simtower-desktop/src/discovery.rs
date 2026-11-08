//! Discovery harness for the desktop's own simulation systems (fork-only). The core crate's
//! harness covers traffic and the tower economy; hotel service, housekeeping dispatch, the
//! finance ledger, star-rating bookkeeping, recycling, parking, VIP visits and quarterly
//! evaluations live in `App`, so this drives a real `App` headlessly: blank textures and muted
//! sounds stand in for the original assets, and `sim_frame` replays the simulation half of
//! `App::update` in its order.
//!
//! Gaps: fire and bomb decisions read the keyboard and mouse, so a pending one is cleared
//! rather than answered; drawing, input and sound are not exercised.

use std::collections::BTreeMap;

use super::*;
use simtower_core::{FacilityOccupancy, MAX_SIMULATED_PEOPLE};

const FRAME_SECONDS: f32 = 1.0 / 60.0;
const PRESENCE_LIMIT_MINUTES: u64 = 2 * 24 * 60;
/// Rooms dirty longer than this become infested at the next 17:00 check.
const DIRTY_DAYS_BEFORE_INFESTATION: u32 = 3;

fn blank_texture() -> Texture2D {
    // An unmanaged handle: no GL context is needed to make it or drop it.
    Texture2D::from_miniquad_texture(macroquad::miniquad::TextureId::from_raw_id(
        macroquad::miniquad::RawId::OpenGl(0),
    ))
}

fn blank_sprite() -> Sprite {
    Sprite {
        resource_id: 0,
        texture: blank_texture(),
        source: Rect::new(0.0, 0.0, 0.0, 0.0),
        alpha: AlphaMask {
            width: 0,
            height: 0,
            opaque: Vec::new(),
        },
    }
}

fn textures<const N: usize>() -> [Texture2D; N] {
    std::array::from_fn(|_| blank_texture())
}

fn sprites<const N: usize>() -> [Sprite; N] {
    std::array::from_fn(|_| blank_sprite())
}

fn headless_assets() -> OriginalAssets {
    OriginalAssets {
        sky: blank_texture(),
        ground: blank_texture(),
        intro_store: blank_texture(),
        intro_title: blank_texture(),
        intro_maxis: blank_texture(),
        rain: textures(),
        clouds: textures(),
        city: blank_texture(),
        tool_palette: blank_texture(),
        tool_palette_selected: blank_texture(),
        tool_palette_disabled: blank_texture(),
        pointer_tools: blank_texture(),
        pointer_tools_selected: blank_texture(),
        elevator_cursor: blank_texture(),
        magnifier_cursor: blank_texture(),
        scaffolding: blank_texture(),
        floor_strip: blank_texture(),
        emergency_stairs: blank_texture(),
        roof_crane: blank_texture(),
        lobby_awning: blank_texture(),
        santa: blank_texture(),
        treasure: blank_texture(),
        people: textures(),
        people_silhouette: textures(),
        queue_people: textures(),
        elevator_cars: blank_texture(),
        elevator_shaft: blank_texture(),
        elevator_numbers: textures(),
        tower_logo: blank_texture(),
        finance_dialog: blank_texture(),
        fire_large: textures(),
        fire_small: blank_texture(),
        fire_helicopter: blank_texture(),
        fire_alert: blank_texture(),
        fire_aftermath: blank_texture(),
        terrorist_portrait: blank_texture(),
        fire_dispatch: blank_texture(),
        vip_arrival: blank_texture(),
        recycling_fill: textures(),
        recycling_truck: blank_texture(),
        facilities: FacilitySprites {
            lobby_first_story: sprites(),
            lobby_second_story: sprites(),
            lobby_third_story: sprites(),
            office_vacant: blank_sprite(),
            office: sprites(),
            condo_states: sprites(),
            hotel_single_vacant: sprites(),
            hotel_single_occupied: sprites(),
            hotel_twin_vacant: sprites(),
            hotel_twin_occupied: sprites(),
            hotel_suite_vacant: sprites(),
            hotel_suite_occupied: sprites(),
            restaurant_vacant: sprites(),
            restaurant_occupied: sprites(),
            fast_food_vacant: sprites(),
            fast_food_occupied: sprites(),
            shop_vacant: sprites(),
            shop_occupied: sprites(),
            cinema_vacant: blank_sprite(),
            cinema_occupied: blank_sprite(),
            party_hall_vacant: blank_sprite(),
            party_hall_occupied: blank_sprite(),
            metro: sprites(),
            parking: sprites(),
            medical: sprites(),
            security: blank_sprite(),
            recycling: blank_sprite(),
            stairs: blank_sprite(),
            escalator: blank_sprite(),
            elevator: blank_sprite(),
            service_elevator: blank_sprite(),
            express_elevator: blank_sprite(),
            housekeeping: blank_sprite(),
            ramp: sprites(),
            cathedral: blank_sprite(),
        },
        sounds: NativeSounds {
            opening_theme: PathBuf::new(),
            opening_theme_process: RefCell::new(None),
            construction: PathBuf::new(),
            lobby_segment: PathBuf::new(),
            demolition: PathBuf::new(),
            no_money: PathBuf::new(),
            payment: PathBuf::new(),
            fire_alert: PathBuf::new(),
            fire_response: PathBuf::new(),
            bomb_threat: PathBuf::new(),
            bomb_armed: PathBuf::new(),
            bomb_explosion: PathBuf::new(),
            elevator_move: PathBuf::new(),
            elevator_open: [PathBuf::new(), PathBuf::new()],
            escalator_move: PathBuf::new(),
            crowd: PathBuf::new(),
            parking_arrival: PathBuf::new(),
            parking_departure: PathBuf::new(),
            muted: Cell::new(true),
        },
    }
}

/// An `App` loaded from save bytes the way the desktop loads them, kept muted and without
/// automatic finance reports (which would pause the clock for a dialog).
fn load_app(bytes: &[u8]) -> Result<App, String> {
    let mut app = App::new(headless_assets());
    app.load_from_bytes(bytes, "discovery.smtower")?;
    app.assets.sounds.set_muted(true);
    app.automatic_finance_reports = false;
    app.view_mode = ViewMode::Edit;
    Ok(app)
}

/// Answers the modal dialogs `App::update` would wait on, as a player dismissing them would.
/// Fire and bomb decisions read the mouse and keyboard, so a pending one is cleared instead.
fn settle_dialogs(app: &mut App) -> Option<&'static str> {
    if app.finance_panel_open {
        app.close_finance_panel();
    }
    if app.promotion_dialog_open {
        app.promotion_dialog_open = false;
        if let Some(speed) = app.promotion_resume_speed.take() {
            app.tower.clock.speed = speed;
        }
    }
    if app.vip_dialog.is_some() {
        app.vip_dialog = None;
        if let Some(speed) = app.vip_dialog_resume_speed.take() {
            app.tower.clock.speed = speed;
        }
    }
    if let Some(event) = app
        .bomb_event
        .filter(|event| event.response == BombResponse::AwaitingDecision)
    {
        app.bomb_event = None;
        restore_emergency_speed(&mut app.tower.clock, event.resume_speed);
        return Some("bomb decision skipped");
    }
    if let Some(event) = app
        .fire_event
        .filter(|event| event.response == FireResponse::AwaitingDecision)
    {
        app.fire_event = None;
        restore_emergency_speed(&mut app.tower.clock, event.resume_speed);
        return Some("fire decision skipped");
    }
    None
}

/// The simulation half of `App::update` for one frame of `dt` real seconds, in its order.
fn sim_frame(app: &mut App, dt: f32) {
    app.finance_ledger.ensure_initialized(app.tower.cash());
    app.finance_ledger.reconcile_external_cash(app.tower.cash());
    let previous_absolute_minute = app.tower.clock.absolute_minute();
    let traffic_scale = match app.tower.clock.speed {
        SimulationSpeed::Paused => 0.0,
        SimulationSpeed::Normal => 1.0,
        SimulationSpeed::Fast => 2.0,
        SimulationSpeed::Triple => 3.0,
        SimulationSpeed::Quintuple => 5.0,
        SimulationSpeed::Tenfold => 10.0,
    };
    let simulation_dt = dt * traffic_scale;
    app.capture_hotel_guest_stays();
    let reachable_lookup = app.traffic.reachable_facility_set().clone();
    let reachable_facilities = reachable_lookup.iter().copied().collect::<Vec<_>>();
    let visitor_counts = app
        .tower
        .facilities()
        .iter()
        .map(|facility| {
            (
                facility.id,
                app.economic_patronage(facility, reachable_lookup.contains(&facility.id)),
            )
        })
        .collect::<Vec<_>>();
    let elevator_cars = app
        .traffic
        .elevators()
        .iter()
        .map(|shaft| (shaft.id, shaft.kind, shaft.cars.len() as u8))
        .collect::<Vec<_>>();
    let income_events = app.tower.advance_time_with_connected_economy(
        dt,
        &visitor_counts,
        &elevator_cars,
        Some(&reachable_facilities),
    );
    app.finance_ledger.record(&income_events);
    record_tenant_finances(
        &mut app.tenant_finances,
        app.tower.clock.day,
        &income_events,
    );
    app.finance_ledger.last_observed_cash = app.tower.cash();
    let closed_quarter =
        quarter_closed_between(previous_absolute_minute, app.tower.clock.absolute_minute());
    app.update_hotel_room_service(previous_absolute_minute, app.tower.clock.absolute_minute());
    app.traffic.advance_after_sync(&app.tower, simulation_dt);
    app.complete_housekeeping_visits();
    app.capture_hotel_guest_stays();
    app.refresh_tower_metrics();
    app.update_recycling_state(previous_absolute_minute, app.tower.clock.absolute_minute());
    app.update_parking_state();
    let _ = app.traffic.take_parking_demand_warning();
    let tower_status = app
        .tower
        .update_tower_status_for_population(app.cached_population);
    if app.tower.tower_status_awarded() {
        app.cached_star_rating = 6;
    } else if let TowerStatusProgress::WeddingInProgress { .. } = tower_status {
    }
    app.update_vip_visit();
    if app.tower.vip_approved() && app.cached_population >= 5_000 {
        app.cached_star_rating = app.tower.record_star_rating(app.cached_population);
    }
    if let Some((year, quarter)) = closed_quarter {
        app.apply_quarterly_evaluation();
        app.finance_ledger
            .close_quarter(year, quarter, app.tower.cash(), app.population_by_kind());
        if app.automatic_finance_reports {
            app.finance_panel_open = true;
        }
    }
    app.update_bomb_event(simulation_dt);
    app.update_fire_event(simulation_dt);
    let current_rating = app.star_rating();
    if current_rating > app.last_star_rating {
        app.promotion_rating = current_rating;
        app.promotion_dialog_open = true;
        app.promotion_resume_speed =
            (app.tower.clock.speed != SimulationSpeed::Paused).then_some(app.tower.clock.speed);
        app.tower.clock.speed = SimulationSpeed::Paused;
    }
    app.last_star_rating = current_rating;
}

#[derive(Default)]
struct Monitor {
    best_rating: u8,
    present_since: HashMap<u64, u64>,
    reported: HashSet<(&'static str, u64)>,
    last_report: Option<(u32, u8)>,
}

impl Monitor {
    fn check(&mut self, app: &App) -> Vec<(&'static str, String)> {
        let mut found = Vec::new();
        let mut flag = |kind: &'static str, detail: String| found.push((kind, detail));
        let (tower, traffic) = (&app.tower, &app.traffic);
        if let Err(error) = tower.validate_save_state() {
            flag("tower fails validation", error);
        }
        if let Err(error) = traffic.validate_save_state(tower) {
            flag("traffic fails validation", error);
        }
        if traffic.people().len() > MAX_SIMULATED_PEOPLE {
            flag("people over the cap", format!("{}", traffic.people().len()));
        }

        // Earned stars are kept (the original never demotes a tower).
        let rating = app.star_rating();
        if rating < self.best_rating && self.reported.insert(("stars", u64::from(rating))) {
            flag(
                "star rating went down",
                format!(
                    "{} -> {rating} at population {}",
                    self.best_rating, app.cached_population
                ),
            );
        }
        self.best_rating = self.best_rating.max(rating);

        // Every cash change since the quarter began is in exactly one ledger line.
        let ledger = &app.finance_ledger;
        let accounted = ledger.period_start_balance + ledger.tenant_income
            - ledger.maintenance
            - ledger.construction_costs
            + ledger.other_income;
        if accounted != ledger.last_observed_cash
            && self
                .reported
                .insert(("ledger", app.tower.clock.absolute_minute()))
        {
            flag(
                "ledger does not add up",
                format!(
                    "start {} + income {} - maintenance {} - construction {} + other {} = {accounted}, but observed cash is {}",
                    ledger.period_start_balance,
                    ledger.tenant_income,
                    ledger.maintenance,
                    ledger.construction_costs,
                    ledger.other_income,
                    ledger.last_observed_cash
                ),
            );
        }
        if let Some(report) = &ledger.last_report
            && self.last_report != Some((report.year, report.quarter))
        {
            self.last_report = Some((report.year, report.quarter));
            let accounted = report.starting_balance + report.total_income
                - report.total_maintenance
                - report.construction_costs
                + report.other_income;
            if accounted != report.ending_balance {
                flag(
                    "quarterly report does not balance",
                    format!(
                        "Y{} Q{}: start {} + income {} - maintenance {} - construction {} + other {} = {accounted}, ending balance {} (off by {})",
                        report.year,
                        report.quarter,
                        report.starting_balance,
                        report.total_income,
                        report.total_maintenance,
                        report.construction_costs,
                        report.other_income,
                        report.ending_balance,
                        report.ending_balance - accounted
                    ),
                );
            }
        }

        // Housekeeping: at most six workers per office, and no room rots while one can reach it.
        let offices = tower
            .facilities()
            .iter()
            .filter(|facility| facility.kind == FacilityKind::Housekeeping)
            .map(|facility| facility.id)
            .collect::<Vec<_>>();
        for &office in &offices {
            let workers = traffic
                .people()
                .iter()
                .filter(|person| person.home_facility_id == office)
                .count();
            if workers > 6 {
                flag(
                    "too many housekeepers",
                    format!("office {office}: {workers}"),
                );
            }
        }
        for room in tower.facilities() {
            let FacilityOccupancy::HotelDirty { since_day } = room.occupancy else {
                continue;
            };
            if tower.clock.day.saturating_sub(since_day) < DIRTY_DAYS_BEFORE_INFESTATION
                || self.reported.contains(&("dirty", room.id))
            {
                continue;
            }
            if let Some(&office) = offices
                .iter()
                .find(|office| traffic.housekeeping_route_available(tower, **office, room.id))
            {
                self.reported.insert(("dirty", room.id));
                flag(
                    "reachable hotel room left dirty",
                    format!(
                        "{:?} {} on floor {} dirty since day {since_day} (now day {}); office {office} can reach it; {}/{} people",
                        room.kind,
                        room.id,
                        room.position.floor,
                        tower.clock.day,
                        traffic.people().len(),
                        MAX_SIMULATED_PEOPLE
                    ),
                );
            }
        }

        // Every stay ends, staff included.
        let now = tower.clock.absolute_minute();
        let mut present = HashSet::new();
        for person in traffic.people() {
            present.insert(person.id);
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
        self.present_since.retain(|id, _| present.contains(id));
        found
    }
}

#[derive(Default)]
struct Report {
    kinds: BTreeMap<&'static str, (usize, Vec<String>)>,
}

impl Report {
    fn add(&mut self, clock: Clock, violations: Vec<(&'static str, String)>) {
        for (kind, detail) in violations {
            let entry = self.kinds.entry(kind).or_default();
            entry.0 += 1;
            if entry.1.len() < 4 {
                entry.1.push(format!(
                    "day {} {:02}:{:02} {detail}",
                    clock.day,
                    clock.minute_of_day / 60,
                    clock.minute_of_day % 60
                ));
            }
        }
    }

    fn print(&self, label: &str) {
        println!("== {label}: {} kinds", self.kinds.len());
        for (kind, (count, details)) in &self.kinds {
            println!("  {kind}: {count}");
            for detail in details {
                println!("      {detail}");
            }
        }
    }
}

/// What a run exercised, so a clean report can be told apart from a run that never got there.
#[derive(Debug, Default)]
struct Coverage {
    rooms_dirtied: usize,
    rooms_cleaned: usize,
    peak_housekeepers: usize,
    quarters_closed: usize,
    vip_visits: usize,
    dirty: HashSet<u64>,
    last_report: Option<(u32, u8)>,
    vip: bool,
}

impl Coverage {
    fn observe(&mut self, app: &App) {
        let dirty = app
            .tower
            .facilities()
            .iter()
            .filter(|facility| facility.is_hotel_dirty())
            .map(|facility| facility.id)
            .collect::<HashSet<_>>();
        self.rooms_dirtied += dirty.difference(&self.dirty).count();
        self.rooms_cleaned += self
            .dirty
            .difference(&dirty)
            .filter(|id| {
                app.tower
                    .facilities()
                    .iter()
                    .any(|facility| facility.id == **id && !facility.is_hotel_dirty())
            })
            .count();
        self.dirty = dirty;
        let offices = app
            .tower
            .facilities()
            .iter()
            .filter(|facility| facility.kind == FacilityKind::Housekeeping)
            .map(|facility| facility.id)
            .collect::<HashSet<_>>();
        let housekeepers = app
            .traffic
            .people()
            .iter()
            .filter(|person| offices.contains(&person.home_facility_id))
            .count();
        self.peak_housekeepers = self.peak_housekeepers.max(housekeepers);
        let report = app
            .finance_ledger
            .last_report
            .as_ref()
            .map(|report| (report.year, report.quarter));
        if report.is_some() && report != self.last_report {
            self.quarters_closed += 1;
            self.last_report = report;
        }
        if app.vip_visit.is_some() && !self.vip {
            self.vip_visits += 1;
        }
        self.vip = app.vip_visit.is_some();
    }
}

fn corpus() -> Vec<(String, Vec<u8>)> {
    let saves = std::env::var("DISCOVERY_SAVES").expect("DISCOVERY_SAVES");
    saves
        .split(':')
        .map(|path| (path.to_owned(), fs::read(path).expect("read save")))
        .collect()
}

fn corpus_speed() -> SimulationSpeed {
    match std::env::var("DISCOVERY_SPEED").as_deref() {
        Ok("Normal") => SimulationSpeed::Normal,
        Ok("Fast") => SimulationSpeed::Fast,
        _ => SimulationSpeed::Tenfold,
    }
}

/// Runs real saves through the desktop's systems with the desktop invariants. Run with
/// DISCOVERY_SAVES=a.smtower:b.smtower DISCOVERY_DAYS=6 cargo test --release
/// -p simtower-desktop --no-default-features desktop_replay -- --ignored --nocapture
#[test]
#[ignore = "needs DISCOVERY_SAVES"]
fn desktop_replay_saves() {
    let days: f32 = std::env::var("DISCOVERY_DAYS").map_or(6.0, |days| days.parse().unwrap());
    for (path, bytes) in corpus() {
        let mut app = load_app(&bytes).expect("load save");
        app.tower.clock.speed = corpus_speed();
        let end = app.tower.clock.absolute_minute() + (days * 24.0 * 60.0) as u64;
        let mut monitor = Monitor::default();
        let mut report = Report::default();
        let mut skipped = BTreeMap::<&str, usize>::new();
        let mut coverage = Coverage {
            last_report: app
                .finance_ledger
                .last_report
                .as_ref()
                .map(|report| (report.year, report.quarter)),
            ..Coverage::default()
        };
        report.add(app.tower.clock, monitor.check(&app));
        while app.tower.clock.absolute_minute() < end {
            if let Some(skip) = settle_dialogs(&mut app) {
                *skipped.entry(skip).or_default() += 1;
            }
            if app.tower.clock.speed == SimulationSpeed::Paused {
                app.tower.clock.speed = corpus_speed();
            }
            sim_frame(&mut app, FRAME_SECONDS);
            report.add(app.tower.clock, monitor.check(&app));
            coverage.observe(&app);
        }
        report.print(&format!("{path} for {days} days"));
        println!(
            "   ended day {} with {} people, {} stars, cash {}; skipped {skipped:?}",
            app.tower.clock.day,
            app.traffic.people().len(),
            app.star_rating(),
            app.tower.cash()
        );
        println!(
            "   rooms dirtied {}, cleaned {}, peak housekeepers {}, quarters closed {}, VIP visits {}",
            coverage.rooms_dirtied,
            coverage.rooms_cleaned,
            coverage.peak_housekeepers,
            coverage.quarters_closed,
            coverage.vip_visits
        );
    }
}

/// The parts of an `App` that shape what happens next: everything a save holds, plus the
/// bookkeeping that lives only in memory.
fn sim_state(app: &App) -> serde_json::Value {
    let mut state = serde_json::to_value(app.save_game()).expect("save encodes");
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
    if let Some(serde_json::Value::Array(items)) = state.get_mut("hotel_guest_stays") {
        items.sort_by_key(|item| item.as_u64());
    }
    state["memory_only"] = serde_json::json!({
        "cached_star_rating": app.cached_star_rating,
        "last_star_rating": app.last_star_rating,
        "cached_population": app.cached_population,
        "fire_event": format!("{:?}", app.fire_event),
    });
    state
}

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

/// Saving and loading mid-game must not change what happens next. Each save runs a while,
/// is saved and loaded into a second app, and both run on; the uninterrupted one first gets
/// the route rebuild that loading performs. Run like `desktop_replay_saves`, with
/// DISCOVERY_WARMUP_FRAMES and DISCOVERY_FRAMES.
#[test]
#[ignore = "needs DISCOVERY_SAVES"]
fn desktop_save_round_trip() {
    let frames = |name: &str, default: usize| {
        std::env::var(name).map_or(default, |value| value.parse().unwrap())
    };
    let (warmup, after) = (
        frames("DISCOVERY_WARMUP_FRAMES", 3600),
        frames("DISCOVERY_FRAMES", 7200),
    );
    let mut failures = Vec::new();
    for (path, bytes) in corpus() {
        let mut original = load_app(&bytes).expect("load save");
        original.tower.clock.speed = corpus_speed();
        let run = |app: &mut App, count: usize| {
            for _ in 0..count {
                settle_dialogs(app);
                if app.tower.clock.speed == SimulationSpeed::Paused {
                    app.tower.clock.speed = corpus_speed();
                }
                sim_frame(app, FRAME_SECONDS);
            }
        };
        run(&mut original, warmup);
        let saved = serde_json::to_vec(&original.save_game()).expect("save encodes");
        let mut reloaded = load_app(&saved).expect("reload");
        original.traffic.sync_with_loaded_tower(&original.tower);
        original.refresh_tower_metrics();
        let at_save = first_difference(&sim_state(&original), &sim_state(&reloaded), "");
        run(&mut original, after);
        run(&mut reloaded, after);
        let later = first_difference(&sim_state(&original), &sim_state(&reloaded), "");
        println!(
            "== {path}\n   right after loading: {at_save:?}\n   {after} frames later: {later:?}"
        );
        if at_save.is_some() || later.is_some() {
            failures.push(path);
        }
    }
    assert!(failures.is_empty(), "{failures:?}");
}
