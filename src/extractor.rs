use crate::sequence::{
    drop_afk_ticks, resolve_region_indices, split_regions_on_gaps, PlayerSequence, RegionEndReason,
    TickData,
};
use crate::world::World;
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

/// Captures tick data during replay
struct DataCapturingWriter {
    tick_data: HashMap<u32, Vec<TickData>>,
    current_tick: i64,
    /// Players who triggered /practice this tick, processed in snap_and_write
    pending_practice_players: Vec<i32>,
}

impl DataCapturingWriter {
    fn new() -> Self {
        Self {
            tick_data: HashMap::new(),
            current_tick: 0,
            pending_practice_players: Vec::new(),
        }
    }
}

impl DemoChatWrite for DataCapturingWriter {
    fn write_chat(&mut self, _msg: &str) -> Result<(), WriteError> {
        Ok(())
    }

    fn write_player_chat(&mut self, player_id: i32, msg: &str) -> Result<(), WriteError> {
        // Detect /practice command
        if msg.starts_with("/practice") {
            self.pending_practice_players.push(player_id);
        }
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
        self.current_tick = tick.snap_tick() as i64;

        snap_buf.clear();
        world.snap(snap_buf);

        for (snap_id, player) in snap_buf.players.iter() {
            // get tracked player (skip if not ready yet)
            let mut tracked_players = world.tracked_players.borrow_mut();
            let Some(tracked) = tracked_players.get_mut(&snap_id.0) else {
                drop(tracked_players);
                assert!(
                    player.tee.is_none(),
                    "player {} has tee but not tracked",
                    snap_id.0
                );
                continue;
            };

            // update current name and team in World (for finish matching)
            tracked.name = player.name.to_string();

            // Detect team change from snap data (e.g., team changed via admin, not /team command)
            if tracked.current_team != player.team {
                tracked.close_region(self.current_tick, RegionEndReason::TeamChange);
                tracked.current_team = player.team;
            }

            tracked.team = player.team;

            // player is tracked, so tee must exist
            let tee = player.tee.as_ref().expect("tracked player must have tee");

            // skip tick if no input received yet
            let Some(input) = &tracked.input else {
                continue;
            };
            let input = input.clone();
            drop(tracked_players);

            let aim_angle = tee.angle.to_num::<f32>();
            let is_frozen = tee.freeze_end > tick;
            let freeze_status = if is_frozen { 1.0 } else { 0.0 };

            let move_dir = input.direction as f32;
            let key_jump = if input.jump { 1.0 } else { 0.0 };
            let key_fire = if input.fire { 1.0 } else { 0.0 };
            let key_hook = if input.hook { 1.0 } else { 0.0 };

            // cursor position (convert from raw to game world units)
            let cursor_x = input.target_x as f32 / 32.0;
            let cursor_y = input.target_y as f32 / 32.0;
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
            let is_grounded = tee.jumps >= 2; // TODO: this is wrong xd
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

            self.tick_data.entry(snap_id.0).or_default().push(tick_data);
        }

        // Process pending /practice commands
        if !self.pending_practice_players.is_empty() {
            let mut tracked_players = world.tracked_players.borrow_mut();

            // Collect teams that triggered practice
            let practice_teams: Vec<i32> = self
                .pending_practice_players
                .iter()
                .filter_map(|&pid| {
                    tracked_players
                        .get(&(pid as u32))
                        .map(|p| p.current_team)
                })
                .collect();

            // Mark all players on those teams as practice
            for player in tracked_players.values_mut() {
                if practice_teams.contains(&player.current_team) {
                    player.set_practice();
                }
            }

            self.pending_practice_players.clear();
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

    // Close regions for still-active players (map ended / server shutdown)
    // Use final_tick + 1 because players are still active at final_tick
    let final_tick = world.current_tick as i64;
    for player in world.tracked_players.borrow_mut().values_mut() {
        player.close_region(final_tick + 1, RegionEndReason::ChangeMap);
    }

    // combine completed players (left during replay) and still-active players
    let all_players = world
        .completed_players
        .into_inner()
        .into_iter()
        .chain(world.tracked_players.into_inner().into_values());

    // convert to PlayerSequence
    let time_of_day = start_time;
    let mut sequences = Vec::new();

    for mut player in all_players {
        // merge tick data from DataCapturingWriter
        if let Some(mut tick_data) = data_writer.tick_data.remove(&player.player_id) {
            player.data.append(&mut tick_data);
        }

        assert!(
            !player.name.is_empty(),
            "player {} has no name",
            player.player_id
        );

        // skip empty sequences
        if player.data.is_empty() {
            continue;
        }

        let start_tick = player.data.first().map(|d| d.tick).unwrap();
        let end_tick = player.data.last().map(|d| d.tick).unwrap();

        // Step 1: Find tick gaps BEFORE AFK removal (these are Spectate gaps)
        let mut regions = player.completed_regions.clone();
        regions = split_regions_on_gaps(regions, &player.data, RegionEndReason::Spectate);

        // Step 2: Remove AFK ticks
        drop_afk_ticks(&mut player.data);

        // Step 3: Find NEW tick gaps AFTER AFK removal (these are AFK gaps)
        regions = split_regions_on_gaps(regions, &player.data, RegionEndReason::Afk);

        // Step 4: Resolve tick boundaries to data array indices
        resolve_region_indices(&mut regions, &player.data);

        sequences.push(PlayerSequence {
            player_name: player.name,
            team: player.team,
            finishes: player.finishes,
            start_tick,
            end_tick,
            time_of_day: time_of_day.clone(),
            map_name: map_name.clone(),
            data: player.data,
            active_regions: regions,
            timeout_code: player.timeout_code,
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
