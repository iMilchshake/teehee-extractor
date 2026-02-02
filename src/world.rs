use crate::sequence::FinishInfo;
use std::collections::{HashMap, HashSet};
use teehistorian_replayer::twgame_core::console::Command;
use teehistorian_replayer::twgame_core::database::Finishes;
use teehistorian_replayer::twgame_core::net_msg::ClNetMessage;
use teehistorian_replayer::twgame_core::replay::{DemoChatPtr, ReplayerTeeInfo};
use teehistorian_replayer::twgame_core::teehistorian::Chunk;
use teehistorian_replayer::twgame_core::twsnap::time::Instant;
use teehistorian_replayer::twgame_core::twsnap::Snap;
use teehistorian_replayer::twgame_core::{Game, Input, Snapper};
use twgame::core::net_msg::Team;
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

/// Wrapper around DdnetReplayerWorld that provides hooks for tracking game events
pub struct World {
    pub world: DdnetReplayerWorld,
    pub finishes: HashMap<String, FinishInfo>,
    pub player_inputs: HashMap<u32, InputState>,
    pub current_tick: SnapTick,
    /// Players who are active (between player_ready and player_leave)
    pub active_players: HashSet<u32>,
}

impl World {
    pub fn new(world: DdnetReplayerWorld) -> Self {
        Self {
            world,
            finishes: HashMap::new(),
            player_inputs: HashMap::new(),
            current_tick: SnapTick::default(),
            active_players: HashSet::new(),
        }
    }
}

// implement Game trait to intercept player inputs
impl Game for World {
    fn player_join(&mut self, id: u32) {
        self.world.player_join(id);
        println!("tick={} id={}: JOIN", self.current_tick, id);
    }

    fn player_ready(&mut self, id: u32) {
        self.active_players.insert(id);
        self.world.player_ready(id);
        println!("tick={} id={}: READY", self.current_tick, id);
    }

    // TODO: this is not called every tick. I believe this is only called if input changes. So its
    // correct to buffer inputs in self.player_inputs, and re-use in future ticks as they do not change.
    fn player_input(&mut self, id: u32, input: &Input) {
        self.player_inputs.insert(id, InputState::from(input));
        self.world.player_input(id, input);
    }

    fn player_leave(&mut self, id: u32) {
        self.active_players.remove(&id);
        self.player_inputs.remove(&id);
        self.world.player_leave(id);
        println!("tick={} id={}: LEAVE", self.current_tick, id);
    }

    fn on_net_msg(&mut self, id: u32, msg: &ClNetMessage) {
        if let ClNetMessage::ClSetTeam(t) = msg {
            // TODO: we can use this to determine if players are currently in spec.
            // while we dont want to extract sequences, we could retain information (e.g. timeout code)
            let spec = match t {
                Team::Spectators => true,
                _ => false,
            };
            println!("tick={} id={}, spec={}", self.current_tick, id, spec);
        }

        self.world.on_net_msg(id, msg);
    }

    fn on_command(&mut self, id: u32, command: &Command) {
        self.world.on_command(id, command);
    }

    fn swap_tees(&mut self, id1: u32, id2: u32) {
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
        self.world.on_teehistorian_chunk(now, chunk);
    }

    fn on_finish(&mut self, now: Instant, finish: &Finishes) {
        // track the finish event as they are not stored in World
        let duration_ticks = match finish {
            Finishes::FinishTee(f) => f.time.ticks(),
            Finishes::FinishTeam(f) => f.time.ticks(),
        };
        let finish_info = FinishInfo {
            tick: now.snap_tick() as i64,
            duration_secs: duration_ticks as f32 / 50.0,
        };
        match finish {
            Finishes::FinishTee(f) => {
                self.finishes.insert(f.name.clone(), finish_info);
            }
            Finishes::FinishTeam(f) => {
                for name in &f.names {
                    self.finishes.insert(name.clone(), finish_info.clone());
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
