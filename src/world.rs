use crate::sequence::{ActiveRegion, FinishInfo, RegionEndReason, TickData};
use std::cell::RefCell;
use std::collections::HashMap;
use teehistorian_replayer::twgame_core::console::Command;
use teehistorian_replayer::twgame_core::database::Finishes;
use teehistorian_replayer::twgame_core::net_msg::ClNetMessage;
use teehistorian_replayer::twgame_core::replay::{DemoChatPtr, ReplayerTeeInfo};
use teehistorian_replayer::twgame_core::teehistorian::Chunk;
use teehistorian_replayer::twgame_core::twsnap::time::Instant;
use teehistorian_replayer::twgame_core::twsnap::Snap;
use teehistorian_replayer::twgame_core::{Game, Input, Snapper};
use twgame::twsnap::time::SnapTick;
use twgame::DdnetReplayerWorld;

/// Player input state wrapper around `Input` to reduce memory usage
#[derive(Debug, Clone, Default)]
pub struct InputState {
    pub direction: i32,
    pub target_x: i32,
    pub target_y: i32,
    pub jump: bool,
    pub fire: bool,
    pub hook: bool,
}

impl From<&Input> for InputState {
    fn from(input: &Input) -> Self {
        Self {
            direction: input.direction,
            target_x: input.target_x,
            target_y: input.target_y,
            jump: (input.jump & 1) != 0,
            fire: input.firing(),
            hook: (input.hook & 1) != 0,
        }
    }
}

/// Tracks all data for an active player during replay
#[derive(Debug, Clone)]
pub struct TrackedPlayer {
    pub player_id: u32,
    pub name: String,
    pub team: i32,
    pub input: Option<InputState>,
    pub data: Vec<TickData>,
    pub timeout_code: Option<String>,
    pub finishes: Vec<FinishInfo>,
    /// Start tick of the current active region
    pub current_region_start: i64,
    /// Team during the current active region
    pub current_team: i32,
    /// Whether practice mode is enabled for current region
    pub current_practice: bool,
    /// Completed active regions
    pub completed_regions: Vec<ActiveRegion>,
}

impl TrackedPlayer {
    /// Close the current region and start a new one.
    /// The end_tick is the tick WHEN the event occurs - the region ends at end_tick - 1.
    pub fn close_region(&mut self, end_tick: i64, reason: RegionEndReason) {
        // Region ends at the tick before the event
        let region_end = end_tick - 1;
        // Only create a region if there's actual content
        if region_end >= self.current_region_start {
            self.completed_regions.push(ActiveRegion::new(
                self.current_region_start,
                region_end,
                self.current_team,
                self.current_practice,
                reason,
            ));
        }
        // New region starts at the event tick
        self.current_region_start = end_tick;
        // Practice is per-team: only reset when leaving the team
        match reason {
            RegionEndReason::TeamChange | RegionEndReason::Leave | RegionEndReason::ChangeMap => {
                self.current_practice = false;
            }
            _ => {} // Kill, SwapTees: practice persists
        }
    }

    /// Set practice mode for the current region
    pub fn set_practice(&mut self) {
        self.current_practice = true;
    }
}

/// Wrapper around DdnetReplayerWorld that provides hooks for tracking game events
pub struct World {
    pub world: DdnetReplayerWorld,
    pub current_tick: SnapTick,
    /// Active players (between player_ready and player_leave)
    pub tracked_players: RefCell<HashMap<u32, TrackedPlayer>>,
    /// Completed player sequences (after player_leave)
    pub completed_players: RefCell<Vec<TrackedPlayer>>,
}

impl World {
    pub fn new(world: DdnetReplayerWorld) -> Self {
        Self {
            world,
            current_tick: SnapTick::default(),
            tracked_players: RefCell::new(HashMap::new()),
            completed_players: RefCell::new(Vec::new()),
        }
    }
}

// implement Game trait to intercept player inputs
impl Game for World {
    fn player_join(&mut self, id: u32) {
        self.world.player_join(id);
    }

    fn player_ready(&mut self, id: u32) {
        let current_tick = self.current_tick as i64;
        self.tracked_players.borrow_mut().insert(
            id,
            TrackedPlayer {
                player_id: id,
                name: String::new(),
                team: 0,
                input: None,
                data: Vec::new(),
                timeout_code: None,
                finishes: Vec::new(),
                current_region_start: current_tick,
                current_team: 0,
                current_practice: false,
                completed_regions: Vec::new(),
            },
        );
        self.world.player_ready(id);
    }

    fn player_input(&mut self, id: u32, input: &Input) {
        if let Some(player) = self.tracked_players.borrow_mut().get_mut(&id) {
            player.input = Some(InputState::from(input));
        }
        self.world.player_input(id, input);
    }

    fn player_leave(&mut self, id: u32) {
        let current_tick = self.current_tick as i64;
        if let Some(mut player) = self.tracked_players.borrow_mut().remove(&id) {
            // Close the final region with Leave reason
            // Use current_tick + 1 because the player is still active at current_tick
            player.close_region(current_tick + 1, RegionEndReason::Leave);
            self.completed_players.borrow_mut().push(player);
        }
        self.world.player_leave(id);
    }

    fn on_net_msg(&mut self, id: u32, msg: &ClNetMessage) {
        if matches!(msg, ClNetMessage::ClKill) {
            let current_tick = self.current_tick as i64;
            if let Some(player) = self.tracked_players.borrow_mut().get_mut(&id) {
                player.close_region(current_tick, RegionEndReason::Kill);
            }
        }
        self.world.on_net_msg(id, msg);
    }

    fn on_command(&mut self, id: u32, command: &Command) {
        let current_tick = self.current_tick as i64;

        match command {
            Command::Kill => {
                if let Some(player) = self.tracked_players.borrow_mut().get_mut(&id) {
                    player.close_region(current_tick, RegionEndReason::Kill);
                }
            }
            // Team changes are detected in snap_and_write from snap data,
            // which is the ground truth. Detecting here too would cause
            // double-detection since the snap lags behind on_command by 1 tick.
            _ => {}
        }

        self.world.on_command(id, command);
    }

    fn swap_tees(&mut self, id1: u32, id2: u32) {
        let current_tick = self.current_tick as i64;

        // Close regions for both players with SwapTees reason
        let mut tracked = self.tracked_players.borrow_mut();
        if let Some(player1) = tracked.get_mut(&id1) {
            player1.close_region(current_tick, RegionEndReason::SwapTees);
        }
        if let Some(player2) = tracked.get_mut(&id2) {
            player2.close_region(current_tick, RegionEndReason::SwapTees);
        }
        drop(tracked);

        self.world.swap_tees(id1, id2);
    }

    fn tick(&mut self, cur_time: Instant) {
        self.world.tick(cur_time);
        self.current_tick = cur_time.snap_tick();
    }

    fn is_empty(&self) -> bool {
        self.world.is_empty()
    }
}

// implement ReplayerChecker trait to intercept finish events
impl teehistorian_replayer::twgame_core::replay::ReplayerChecker for World {
    fn on_teehistorian_header(&mut self, header: &[u8]) {
        self.world.on_teehistorian_header(header);
    }

    fn on_teehistorian_chunk(&mut self, now: Instant, chunk: &Chunk) {
        if let Chunk::ConsoleCommand(cmd) = chunk {
            if cmd.cmd == b"timeout" {
                if let Some(code) = cmd.args.first() {
                    if let Ok(code_str) = std::str::from_utf8(code) {
                        if cmd.cid >= 0 {
                            if let Some(player) =
                                self.tracked_players.borrow_mut().get_mut(&(cmd.cid as u32))
                            {
                                player.timeout_code = Some(code_str.to_string());
                            }
                        }
                    }
                }
            }
        }
        self.world.on_teehistorian_chunk(now, chunk);
    }

    fn on_finish(&mut self, now: Instant, finish: &Finishes) {
        let duration_ticks = match finish {
            Finishes::FinishTee(f) => f.time.ticks(),
            Finishes::FinishTeam(f) => f.time.ticks(),
        };
        let finish_info = FinishInfo {
            tick: now.snap_tick() as i64,
            duration_secs: duration_ticks as f32 / 50.0,
        };

        // match finish by current player name
        let names: Vec<&str> = match finish {
            Finishes::FinishTee(f) => vec![&f.name],
            Finishes::FinishTeam(f) => f.names.iter().map(|s| s.as_str()).collect(),
        };
        for name in names {
            for player in self.tracked_players.borrow_mut().values_mut() {
                if player.name == name {
                    player.finishes.push(finish_info.clone());
                }
            }
        }

        self.world.on_finish(now, finish);
    }

    fn check_tees(
        &mut self,
        cur_time: Instant,
        tees: &[Option<ReplayerTeeInfo>],
        demo: DemoChatPtr,
    ) {
        self.world.check_tees(cur_time, tees, demo);
    }

    fn finalize(&mut self) {
        self.world.finalize();
    }
}

impl Snapper for World {
    fn snap(&self, snapshot: &mut Snap) {
        self.world.snap(snapshot);
    }
}
