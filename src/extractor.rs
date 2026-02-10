use crate::sequence::{
    drop_afk_ticks, resolve_region_indices, split_regions_on_gaps, PlayerSequence, RegionEndReason,
    TickData,
};
use crate::world::World;
use anyhow::{Context, Result};
use log::{debug, warn};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
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

/// Maximum legit distance (in tile units) per tick before triggering teleport check.
/// ~3 tiles. Squared for fast comparison.
const TELEPORT_DIST_SQ_THRESHOLD: f32 = 3.0 * 3.0;

/// Captures tick data during replay
struct DataCapturingWriter {
    current_tick: i64,
    /// Players who triggered /practice this tick, processed in snap_and_write
    pending_practice_players: Vec<i32>,
    /// Teams that have practice mode enabled
    practice_teams: HashSet<i32>,
    /// sv_rescue=1 in server config: all players can /r without practice.
    /// When true, all players are treated as having practice enabled.
    // TODO: this is overly broad — players only have access to /r, not full practice.
    // in the future we should check if players actually use /r or /rescue rather than
    // blanket-flagging everyone as practice.
    sv_rescue: bool,
    /// Map data for teleporter tile lookups
    map: Arc<Map>,
    /// Teehistorian file path for log messages
    file_path: String,
}

impl DataCapturingWriter {
    fn new(map: Arc<Map>, sv_rescue: bool, file_path: String) -> Self {
        Self {
            current_tick: 0,
            pending_practice_players: Vec::new(),
            practice_teams: HashSet::new(),
            sv_rescue,
            map,
            file_path,
        }
    }

    /// Check if a large position jump is caused by a teleporter.
    /// Checks both: tele entrance near prev_pos, and tele exit near current_pos.
    fn is_map_teleport(&self, prev_x: f32, prev_y: f32, cur_x: f32, cur_y: f32) -> bool {
        // check if prev pos is near a tele entrance tile (5x5 grid to account for high-speed entry)
        // positions are in tile units, truncate to tile index (x.5 = center of tile x)
        let tile_x = prev_x as i32;
        let tile_y = prev_y as i32;
        for dy in -2..=2 {
            for dx in -2..=2 {
                if self
                    .map
                    .get_tele_tile(vek::Vec2::new(tile_x + dx, tile_y + dy))
                    .is_some()
                {
                    return true;
                }
            }
        }

        // check if current pos is near a tele exit position
        let cur_tile_x = cur_x as i32;
        let cur_tile_y = cur_y as i32;
        for exits in &self.map.tele_outs {
            for exit in exits {
                if (exit.x - cur_tile_x).abs() <= 1 && (exit.y - cur_tile_y).abs() <= 1 {
                    return true;
                }
            }
        }
        for exits in &self.map.tele_checkpoint_outs {
            for exit in exits {
                if (exit.x - cur_tile_x).abs() <= 1 && (exit.y - cur_tile_y).abs() <= 1 {
                    return true;
                }
            }
        }

        false
    }

    /// Check if a position is exactly on a spawn point (death-tile kill or spawn reassignment).
    fn is_spawn_position(&self, x: f32, y: f32) -> bool {
        let tile_x = x as i32;
        let tile_y = y as i32;
        for spawn_set in &self.map.spawn_points {
            for sp in spawn_set {
                if sp.x == tile_x && sp.y == tile_y {
                    return true;
                }
            }
        }
        false
    }
}

impl DemoChatWrite for DataCapturingWriter {
    fn write_chat(&mut self, _msg: &str) -> Result<(), WriteError> {
        Ok(())
    }

    fn write_player_chat(&mut self, player_id: i32, msg: &str) -> Result<(), WriteError> {
        debug!("chat: player {} at tick {}: {}", player_id, self.current_tick, msg);
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

            // detect team change from snap data (e.g., team changed via admin, not /team command)
            if tracked.current_team != player.team {
                let old_team = tracked.current_team;
                tracked.close_region(self.current_tick, RegionEndReason::TeamChange);
                tracked.current_team = player.team;
                // inherit practice state from new team
                tracked.current_practice = self.practice_teams.contains(&player.team);

                // remove old team from practice set if no one is left on it
                if self.practice_teams.contains(&old_team) {
                    let anyone_left = snap_buf
                        .players
                        .iter()
                        .any(|(sid, p)| sid.0 != snap_id.0 && p.team == old_team);
                    if !anyone_left {
                        self.practice_teams.remove(&old_team);
                    }
                }
            }

            tracked.team = player.team;

            let tee = player.tee.as_ref().expect("tracked player must have tee");

            // skip tick if no input received yet
            let Some(input) = &tracked.input else {
                continue;
            };
            let input = input.clone();
            let prev_pos = tracked.prev_pos;
            let is_practice = tracked.current_practice;
            let killed_this_tick = tracked.killed_this_tick;
            tracked.killed_this_tick = false;
            drop(tracked_players);

            let pos_x = tee.pos.x.to_num::<f32>();
            let pos_y = tee.pos.y.to_num::<f32>();

            // teleport detection: large position change -> check cause
            if let Some((prev_x, prev_y)) = prev_pos {
                let dx = pos_x - prev_x;
                let dy = pos_y - prev_y;
                let dist_sq = dx * dx + dy * dy;
                if dist_sq > TELEPORT_DIST_SQ_THRESHOLD && killed_this_tick {
                    let dist = dist_sq.sqrt();
                    debug!(
                        "kill-respawn: {} at tick {} on map '{}' jumped {dist:.1} tiles \
                         from ({prev_x:.1}, {prev_y:.1}) to ({pos_x:.1}, {pos_y:.1})",
                        player.name, self.current_tick, world.map_name,
                    );
                }
                if dist_sq > TELEPORT_DIST_SQ_THRESHOLD && !killed_this_tick {
                    let is_tile_tp = self.is_map_teleport(prev_x, prev_y, pos_x, pos_y);
                    // assume /r command caused the teleport when sv_rescue is enabled
                    let is_spawn_tp = self.is_spawn_position(pos_x, pos_y);
                    if is_tile_tp || is_practice || self.sv_rescue {
                        world
                            .tracked_players
                            .borrow_mut()
                            .get_mut(&snap_id.0)
                            .expect("player disappeared mid-tick")
                            .close_region(self.current_tick, RegionEndReason::Teleport);
                    } else if is_spawn_tp {
                        // death-tile kill or spawn reassignment — no kill event in teehistorian
                        world
                            .tracked_players
                            .borrow_mut()
                            .get_mut(&snap_id.0)
                            .expect("player disappeared mid-tick")
                            .close_region(self.current_tick, RegionEndReason::Kill);
                    } else {
                        let dist = dist_sq.sqrt();
                        warn!(
                            "unknown large position jump ({dist:.1} tiles) for '{}' at tick {} \
                             on map '{}' from ({prev_x:.1}, {prev_y:.1}) to ({pos_x:.1}, {pos_y:.1}) \
                             in {}",
                            player.name, self.current_tick, world.map_name,
                            self.file_path,
                        );
                        world
                            .tracked_players
                            .borrow_mut()
                            .get_mut(&snap_id.0)
                            .expect("player disappeared mid-tick")
                            .close_region(self.current_tick, RegionEndReason::UnknownTeleport);
                    }
                }
            }

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
                pos_x,
                pos_y,
                vel_x: tee.vel.x.to_num::<f32>() / 32.0,
                vel_y: tee.vel.y.to_num::<f32>() / 32.0,
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

            // push directly into TrackedPlayer so data follows it on player_leave
            let mut tracked_players = world.tracked_players.borrow_mut();
            let tracked = tracked_players
                .get_mut(&snap_id.0)
                .expect("player disappeared mid-tick");
            tracked.data.push(tick_data);
            tracked.prev_pos = Some((pos_x, pos_y));
            drop(tracked_players);
        }

        // process pending /practice commands
        if !self.pending_practice_players.is_empty() {
            let mut tracked_players = world.tracked_players.borrow_mut();

            // add teams to practice set (team 0 = no team, can't practice)
            for &pid in &self.pending_practice_players {
                if let Some(player) = tracked_players.get(&(pid as u32)) {
                    if player.current_team != 0 {
                        self.practice_teams.insert(player.current_team);
                    }
                }
            }
            self.pending_practice_players.clear();

            // mark all current players on newly-practice teams
            for player in tracked_players.values_mut() {
                if self.practice_teams.contains(&player.current_team) {
                    player.set_practice();
                }
            }
        }

        Ok(())
    }

    fn chat(&mut self) -> &mut (dyn DemoChatWrite + 'static) {
        self
    }
}

/// Extract player sequences from a teehistorian file using the replayer.
/// `afk_ticks`: if Some, remove AFK periods longer than this many ticks. If None, skip AFK removal.
pub fn extract_sequences(
    teehistorian_path: &Path,
    maps_dir: &Path,
    afk_ticks: Option<usize>,
) -> Result<Vec<PlayerSequence>> {
    let file = File::open(teehistorian_path)
        .with_context(|| format!("Failed to open file: {}", teehistorian_path.display()))?;

    let mut th_stream = ThCompat::parse(ThBufReader::new(file))?;

    // parse teehistorian header to extract metadata
    let header_raw = th_stream.header()?;
    let th_header = ThHeader::from_buf(header_raw);

    debug!("processing: {}", teehistorian_path.display());
    let map_name = th_header.map_name.clone();
    let start_time = th_header.start_time.clone();
    let map_sha256 = th_header.map_sha256.clone();

    // load the map from the maps directory
    let map_data = load_map_from_dir(maps_dir, &map_name, map_sha256.as_deref())?;

    let mut parsed_map = twmap::TwMap::parse(&map_data)?;
    let map = Map::try_from(&mut parsed_map).map_err(|e| anyhow::anyhow!(e))?;
    let map = Arc::new(map);
    let map_for_writer = Arc::clone(&map);

    // create the replayer world with finish tracking
    let inner_world = DdnetReplayerWorld::new(map, false);
    let mut world = World::new(inner_world, map_name.clone());

    // create our custom data capturing writer
    let sv_rescue = th_header.config.get("sv_rescue").map_or(false, |v| v == "1");
    let mut data_writer = DataCapturingWriter::new(
        map_for_writer,
        sv_rescue,
        teehistorian_path.display().to_string(),
    );

    // replay the game and capture data
    let replayer = ThReplayer::new(header_raw, &mut world);
    replayer.validate(&mut world, &mut th_stream, Some(&mut data_writer));

    // close regions for still-active players (map ended / server shutdown)
    // use final_tick + 1 because players are still active at final_tick
    let final_tick = world.current_tick as i64;
    for player in world.tracked_players.borrow_mut().values_mut() {
        player.close_region(final_tick + 1, RegionEndReason::ChangeMap);
    }

    // extract players and drop the replayer world + map to free memory early
    let completed = world.completed_players.into_inner();
    let tracked = world.tracked_players.into_inner().into_values();
    drop(world.world); // free DdnetReplayerWorld + Arc<Map> before building sequences
    drop(th_stream); // free the teehistorian reader
    drop(data_writer); // free the writer's Arc<Map> clone
    let all_players = completed.into_iter().chain(tracked);

    // convert to PlayerSequence
    let time_of_day = start_time;
    let mut sequences = Vec::new();

    for mut player in all_players {
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

        // step 1: find tick gaps BEFORE AFK removal (these are Spectate gaps)
        let mut regions = player.completed_regions.clone();
        regions = split_regions_on_gaps(regions, &player.data, RegionEndReason::Spectate);

        // step 2: remove AFK ticks (if enabled)
        if let Some(threshold) = afk_ticks {
            drop_afk_ticks(&mut player.data, threshold);

            // step 3: find new tick gaps after AFK removal (these are AFK gaps)
            regions = split_regions_on_gaps(regions, &player.data, RegionEndReason::Afk);
        }

        // step 4: resolve tick boundaries to data array indices
        resolve_region_indices(&mut regions, &player.data);

        // step 5: drop regions with no data remaining after AFK removal.
        // this happens when a region's entire tick range was AFK (e.g. player
        // idle before leaving), resulting in start_idx/end_idx both being None.
        regions.retain(|r| r.start_idx.is_some());

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
