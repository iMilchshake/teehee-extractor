use crate::sequence::{PlayerSequence, TickData};
use crate::world::{InputState, World};
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;
use teehistorian_replayer::teehistorian::{ThBufReader, ThCompat, ThStream};
use teehistorian_replayer::twgame_core::replay::DemoChatWrite;
use teehistorian_replayer::twgame_core::twsnap::compat::ddnet::WriteError;
use teehistorian_replayer::twgame_core::twsnap::enums::{ActiveWeapon, HookState};
use teehistorian_replayer::twgame_core::twsnap::time::Instant;
use teehistorian_replayer::twgame_core::twsnap::Snap;
use teehistorian_replayer::twgame_core::{replay::DemoWrite, Snapper};
use teehistorian_replayer::ThReplayer;
use twgame::{DdnetReplayerWorld, Map, ThHeader};

/// Stores only subset of PlayerSequence data that is used for export.
struct FinishedSequence {
    player_name: String,
    team: i32,
    data: Vec<TickData>,
}

/// Custom demo writer that captures game state instead of writing to file.
struct DataCapturingWriter {
    // hashmaps use snap id as key
    active_sequences: HashMap<u32, Vec<TickData>>,
    active_player_info: HashMap<u32, (String, i32)>, // (name, team)
    start_tick: HashMap<u32, i64>,

    finished_sequences: Vec<FinishedSequence>,
    current_tick: i64,
    snap_count: u64,
}

impl DataCapturingWriter {
    fn new() -> Self {
        Self {
            active_sequences: HashMap::new(),
            active_player_info: HashMap::new(),
            start_tick: HashMap::new(),
            finished_sequences: Vec::new(),
            current_tick: 0,
            snap_count: 0,
        }
    }
}

impl DemoChatWrite for DataCapturingWriter {
    fn write_chat(&mut self, _msg: &str) -> Result<(), WriteError> {
        Ok(())
    }

    fn write_player_chat(&mut self, _player_id: i32, _msg: &str) -> Result<(), WriteError> {
        Ok(())
    }
}

impl DemoWrite<World> for DataCapturingWriter {
    fn snap_and_write(
        &mut self,
        tick: Instant,
        world: &World,
        snap_buf: &mut Snap,
    ) -> Result<(), WriteError> {
        self.snap_count += 1;
        self.current_tick = tick.snap_tick() as i64;

        // prepare snap buffer / world
        snap_buf.clear();
        world.snap(snap_buf);

        let mut seen_this_tick = std::collections::HashSet::new();
        for (snap_id, player) in snap_buf.players.iter() {
            seen_this_tick.insert(snap_id.0);

            match self.active_player_info.entry(snap_id.0) {
                std::collections::hash_map::Entry::Vacant(v) => {
                    v.insert((player.name.to_string(), player.team));
                }
                std::collections::hash_map::Entry::Occupied(mut o) => {
                    // keep the first non-empty name seen (ignore renames)
                    if !player.name.is_empty() && o.get().0.is_empty() {
                        o.get_mut().0 = player.name.to_string();
                    }
                }
            }

            if let Some(tee) = &player.tee {
                let aim_angle = tee.angle.to_num::<f32>();
                let is_frozen = tee.freeze_end > tick;
                let freeze_status = if is_frozen { 1.0 } else { 0.0 };

                // input state for this player (use defaults if not yet received)
                let default_input = InputState::default();
                let input = world
                    .player_inputs
                    .get(&snap_id.0)
                    .unwrap_or(&default_input);
                let move_dir = input.direction as f32;
                let key_jump = if input.jump { 1.0 } else { 0.0 };
                let key_fire = if input.fire { 1.0 } else { 0.0 };
                let key_hook = if input.hook { 1.0 } else { 0.0 };

                // cursor position (raw and polar coordinates)
                let cursor_x = input.target_x as f32;
                let cursor_y = input.target_y as f32;
                let aim_distance = (cursor_x * cursor_x + cursor_y * cursor_y).sqrt();

                // hook state
                let hook_grabbed = tee.hook_state == HookState::Grabbed;
                let (hook_pos_x, hook_pos_y) = if hook_grabbed {
                    (
                        tee.hook_pos.x.to_num::<f32>(),
                        tee.hook_pos.y.to_num::<f32>(),
                    )
                } else {
                    (0.0, 0.0) // if hook is not actively grabbing, set hook position to (0, 0)
                };

                // weapon selection
                let is_hammer = tee.weapon == ActiveWeapon::Hammer;
                let is_gun = tee.weapon == ActiveWeapon::Pistol;
                let is_other_weapon = !is_hammer && !is_gun;

                // jump state
                let is_grounded = tee.jumps >= 2;
                let can_jump = tee.jumps > 0;

                let tick_data = TickData {
                    tick: self.current_tick,
                    pos_x: tee.pos.x.to_num::<f32>(),
                    pos_y: tee.pos.y.to_num::<f32>(),
                    vel_x: tee.vel.x.to_num::<f32>(),
                    vel_y: tee.vel.y.to_num::<f32>(),
                    cursor_x,
                    cursor_y,
                    aim_angle,
                    aim_distance,
                    move_dir,
                    key_jump,
                    key_fire,
                    key_hook,
                    is_grounded: if is_grounded { 1.0 } else { 0.0 },
                    freeze_status,
                    hook_grabbed: if hook_grabbed { 1.0 } else { 0.0 },
                    hook_pos_x,
                    hook_pos_y,
                    is_hammer: if is_hammer { 1.0 } else { 0.0 },
                    is_gun: if is_gun { 1.0 } else { 0.0 },
                    is_other_weapon: if is_other_weapon { 1.0 } else { 0.0 },
                    jumps_remaining: tee.jumps as f32,
                    can_jump: if can_jump { 1.0 } else { 0.0 },
                };

                self.active_sequences
                    .entry(snap_id.0)
                    .or_default()
                    .push(tick_data);
            }
        }

        for id in &seen_this_tick {
            self.start_tick.entry(*id).or_insert(self.current_tick);
        }

        // finish sequences for ids that disappeared (player left or went to spectate?)

        // collect finished ids first (required for borrow checker)
        let mut finished_ids = Vec::new();
        for (id, start) in self.start_tick.iter() {
            if !seen_this_tick.contains(id) {
                let (name, _team) = self
                    .active_player_info
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| (format!("player_{id}"), 0));
                println!(
                    "id={}: [{}, {}], name={}",
                    id,
                    start,
                    self.current_tick - 1,
                    name
                );
                finished_ids.push(*id);
            }
        }
        for id in finished_ids {
            self.start_tick.remove(&id);

            let (player_name, team) = self
                .active_player_info
                .remove(&id)
                .unwrap_or_else(|| (format!("player_{id}"), 0));

            if let Some(data) = self.active_sequences.remove(&id) {
                self.finished_sequences.push(FinishedSequence {
                    player_name,
                    team,
                    data,
                });
            }
        }

        Ok(())
    }

    fn chat(&mut self) -> &mut (dyn DemoChatWrite + 'static) {
        self
    }
}

/// Extract player sequences from a teehistorian file using the replayer.
pub fn extract_sequences(teehistorian_path: &Path, maps_dir: &Path) -> Result<Vec<PlayerSequence>> {
    let file = File::open(teehistorian_path)
        .with_context(|| format!("Failed to open file: {}", teehistorian_path.display()))?;

    let mut th_stream = ThCompat::parse(ThBufReader::new(file))?;

    // parse teehistorian header to extract metadata
    let header_raw = th_stream.header()?;
    let th_header = ThHeader::from_buf(header_raw);

    let map_name = th_header.map_name.clone();
    let start_time = th_header.start_time.clone();
    let map_sha256 = th_header.map_sha256.clone();

    println!("  Map: {map_name}");
    println!("  Start time: {start_time}");
    if let Some(ref sha256) = map_sha256 {
        println!("  Map SHA256: {sha256}");
    }

    // load the map from the maps directory
    println!("  Loading map from: {}", maps_dir.display());
    let map_data = load_map_from_dir(maps_dir, &map_name, map_sha256.as_deref())?;
    println!("  Map loaded successfully ({} bytes)", map_data.len());

    let mut parsed_map = twmap::TwMap::parse(&map_data)?;
    let map = Map::try_from(&mut parsed_map).map_err(|e| anyhow::anyhow!(e))?;
    let map = Arc::new(map);

    // create the replayer world with finish tracking
    let inner_world = DdnetReplayerWorld::new(map, false);
    let mut world = World::new(inner_world);

    // create our custom data capturing writer
    let mut data_writer = DataCapturingWriter::new();

    // replay the game and capture data
    println!("  Processing teehistorian file...");
    let replayer = ThReplayer::new(header_raw, &mut world);
    replayer.validate(&mut world, &mut th_stream, Some(&mut data_writer));

    // extract finish information from the world wrapper
    let finishes = world.finishes;

    // convert captured data to PlayerSequence
    let time_of_day = start_time;

    // start with finished sequences (players who left during replay)
    let mut sequences: Vec<PlayerSequence> = data_writer
        .finished_sequences
        .into_iter()
        .map(|seq| {
            let start_tick = seq.data.first().map(|d| d.tick).unwrap_or(0);
            let end_tick = seq.data.last().map(|d| d.tick).unwrap_or(0);
            let finish = finishes.get(&seq.player_name).cloned();

            PlayerSequence {
                player_name: seq.player_name,
                team: seq.team,
                finish,
                start_tick,
                end_tick,
                time_of_day: time_of_day.clone(),
                map_name: map_name.clone(),
                data: seq.data,
            }
        })
        .collect();

    // add any still-active sequences (players still in game at end of replay)
    for (player_id, data) in data_writer.active_sequences {
        let start_tick = data.first().map(|d| d.tick).unwrap_or(0);
        let end_tick = data.last().map(|d| d.tick).unwrap_or(0);

        let (player_name, team) = data_writer
            .active_player_info
            .get(&player_id)
            .cloned()
            .unwrap_or_else(|| (format!("player_{player_id}"), 0));

        let finish = finishes.get(&player_name).cloned();

        sequences.push(PlayerSequence {
            player_name,
            team,
            finish,
            start_tick,
            end_tick,
            time_of_day: time_of_day.clone(),
            map_name: map_name.clone(),
            data,
        });
    }

    Ok(sequences)
}

/// Load a map from the maps directory and optionally verify its hash.
fn load_map_from_dir(
    maps_dir: &Path,
    map_name: &str,
    expected_sha256: Option<&str>,
) -> Result<Vec<u8>> {
    // load the map file
    let map_path = maps_dir.join(format!("{map_name}.map"));
    let map_data = std::fs::read(&map_path)
        .with_context(|| format!("Failed to load map from: {}", map_path.display()))?;

    // verify SHA256 hash if provided in the teehistorian header
    if let Some(expected_hash) = expected_sha256 {
        let mut hasher = Sha256::new();
        hasher.update(&map_data);
        let actual_hash = format!("{:x}", hasher.finalize());

        if actual_hash != expected_hash {
            anyhow::bail!(
                "Map hash mismatch for {map_name}:\n  Expected: {expected_hash}\n  Actual:   {actual_hash}"
            );
        }
    }

    Ok(map_data)
}
