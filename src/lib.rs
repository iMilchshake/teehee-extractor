mod world;

use anyhow::{Context, Result};
use core::panic;
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;
use teehistorian_replayer::teehistorian::{ThBufReader, ThCompat, ThStream};
use teehistorian_replayer::twgame_core::replay::{DemoChatWrite, DemoWrite};
use teehistorian_replayer::twgame_core::twsnap::compat::ddnet::WriteError;
use teehistorian_replayer::twgame_core::twsnap::time::Instant;
use teehistorian_replayer::twgame_core::twsnap::Snap;
use teehistorian_replayer::twgame_core::Snapper;
use teehistorian_replayer::ThReplayer;
use twgame::twsnap::SnapId;
use twgame::{DdnetReplayerWorld, Map, ThHeader};
use world::World;

/// Data recorded for each game tick
#[derive(Debug, Clone)]
pub struct TickData {
    pub tick: i64,
    pub pos_x: f32,
    pub pos_y: f32,
    pub aim_angle: f32,
    pub freeze_status: f32,
}

/// Information about when and how a player finished the map
#[derive(Debug, Clone)]
pub struct FinishInfo {
    pub tick: i64,          // Game tick when finish occurred
    pub duration_secs: f32, // Run duration in seconds
}

/// Complete sequence of a player from join to leave
#[derive(Debug, Clone)]
pub struct PlayerSequence {
    pub player_name: String,
    pub team: i32,
    pub finish: Option<FinishInfo>,
    pub start_tick: i64,
    pub end_tick: i64,
    pub time_of_day: String,
    pub map_name: String,
    pub data: Vec<TickData>,
}

/// Custom demo writer that captures game state instead of writing to file
struct DataCapturingWriter {
    sequences: HashMap<u32, Vec<TickData>>,
    player_info: HashMap<u32, (String, i32)>, // (name, team)
    current_tick: i64,
    snap_count: u64,

    start_tick: HashMap<u32, i64>,
}

impl DataCapturingWriter {
    fn new() -> Self {
        Self {
            sequences: HashMap::new(),
            player_info: HashMap::new(),
            current_tick: 0,
            snap_count: 0,
            start_tick: HashMap::new(),
        }
    }
}

// Implement DemoChatWrite (required by DemoWrite)
impl DemoChatWrite for DataCapturingWriter {
    fn write_chat(&mut self, _msg: &str) -> Result<(), WriteError> {
        Ok(())
    }

    fn write_player_chat(&mut self, _player_id: i32, _msg: &str) -> Result<(), WriteError> {
        Ok(())
    }
}

impl DemoWrite<DdnetReplayerWorld> for DataCapturingWriter {
    fn snap_and_write(
        &mut self,
        _tick: Instant,
        _world: &DdnetReplayerWorld,
        _snap_buf: &mut Snap,
    ) -> Result<(), WriteError> {
        panic!("not implemented!");
    }

    fn chat(&mut self) -> &mut (dyn DemoChatWrite + 'static) {
        self
    }
}

// Implement DemoWrite for World to enable demo writing
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
            if !player.name.is_empty() {
                self.player_info.entry(snap_id.0).or_insert_with(|| {
                    println!(
                        "tick={} getting player name for snap_id={}, name={}",
                        tick, snap_id.0, player.name
                    );
                    (player.name.to_string(), player.team)
                });
            }

            // dbg!(&snap_id.0, &player);
            seen_this_tick.insert(snap_id.0);

            match self.player_info.entry(snap_id.0) {
                std::collections::hash_map::Entry::Vacant(_) => {
                    println!("tick={}, new snap id = {}", &tick, snap_id.0);
                }
                std::collections::hash_map::Entry::Occupied(o) => {
                    // nothing
                }
            }

            self.player_info
                .entry(snap_id.0)
                .or_insert_with(|| (player.name.to_string(), player.team));

            // dbg!(&self.player_info);

            if let Some(tee) = &player.tee {
                let aim_angle = tee.angle.to_num::<f32>();
                let is_frozen = tee.freeze_end > tick;
                let freeze_status = if is_frozen { 1.0 } else { 0.0 };

                let tick_data = TickData {
                    tick: self.current_tick,
                    pos_x: tee.pos.x.to_num::<f32>(),
                    pos_y: tee.pos.y.to_num::<f32>(),
                    aim_angle,
                    freeze_status,
                };

                self.sequences
                    .entry(snap_id.0)
                    .or_insert_with(Vec::new)
                    .push(tick_data);
            }
        }

        for id in &seen_this_tick {
            self.start_tick.entry(*id).or_insert(self.current_tick);
        }
        // 3) finish runs for ids that disappeared
        let mut finished = Vec::new();
        for (id, start) in self.start_tick.iter() {
            if !seen_this_tick.contains(id) {
                println!(
                    "id={}: [{}, {}], name={}",
                    id,
                    start,
                    self.current_tick - 1,
                    self.player_info.get(id).unwrap().0
                );
                finished.push(*id);
            }
        }

        for id in finished {
            self.start_tick.remove(&id);
        }

        Ok(())
    }

    fn chat(&mut self) -> &mut (dyn DemoChatWrite + 'static) {
        self
    }
}

/// Extract player sequences from a teehistorian file using the replayer
pub fn extract_sequences(teehistorian_path: &Path, maps_dir: &Path) -> Result<Vec<PlayerSequence>> {
    let file = File::open(teehistorian_path)
        .with_context(|| format!("Failed to open file: {}", teehistorian_path.display()))?;

    let mut th_stream = ThCompat::parse(ThBufReader::new(file))?;

    // Parse teehistorian header to extract metadata
    let header_raw = th_stream.header()?;
    let th_header = ThHeader::from_buf(header_raw);

    let map_name = th_header.map_name.clone();
    let start_time = th_header.start_time.clone();
    let map_sha256 = th_header.map_sha256.clone();

    println!("  Map: {}", map_name);
    println!("  Start time: {}", start_time);
    if let Some(ref sha256) = map_sha256 {
        println!("  Map SHA256: {}", sha256);
    }

    // Load the map from the maps directory
    println!("  Loading map from: {}", maps_dir.display());
    let map_data = load_map_from_dir(maps_dir, &map_name, map_sha256.as_deref())?;
    println!("  Map loaded successfully ({} bytes)", map_data.len());

    let mut parsed_map = twmap::TwMap::parse(&map_data)?;
    let map = Map::try_from(&mut parsed_map).map_err(|e| anyhow::anyhow!(e))?;
    let map = Arc::new(map);

    // Create the replayer world with finish tracking
    let inner_world = DdnetReplayerWorld::new(map, false);
    let mut world = World::new(inner_world);

    // Create our custom data capturing writer
    let mut data_writer = DataCapturingWriter::new();

    // Replay the game and capture data
    println!("  Processing teehistorian file...");
    let replayer = ThReplayer::new(header_raw, &mut world);
    replayer.validate(&mut world, &mut th_stream, Some(&mut data_writer));

    // Extract finish information and player names from the world wrapper
    let finishes = world.finishes;
    let player_names = world.player_names;

    // Convert captured data to PlayerSequence
    let time_of_day = start_time;
    let sequences: Vec<PlayerSequence> = data_writer
        .sequences
        .into_iter()
        .map(|(player_id, data)| {
            let start_tick = data.first().map(|d| d.tick).unwrap_or(0);
            let end_tick = data.last().map(|d| d.tick).unwrap_or(0);

            // Get player name from net messages (ClStartInfo), fall back to snap data or placeholder
            let player_name = player_names
                .get(&player_id)
                .cloned()
                .or_else(|| data_writer.player_info.get(&player_id).map(|(n, _)| n.clone()))
                .unwrap_or_else(|| format!("player_{}", player_id));

            // Get team from snap data
            let team = data_writer
                .player_info
                .get(&player_id)
                .map(|(_, t)| *t)
                .unwrap_or(0);

            // Look up finish info for this player
            let finish = finishes.get(&player_name).cloned();

            PlayerSequence {
                player_name,
                team,
                finish,
                start_tick,
                end_tick,
                time_of_day: time_of_day.clone(),
                map_name: map_name.clone(),
                data,
            }
        })
        .collect();

    Ok(sequences)
}

/// Load a map from the maps directory and optionally verify its hash
fn load_map_from_dir(
    maps_dir: &Path,
    map_name: &str,
    expected_sha256: Option<&str>,
) -> Result<Vec<u8>> {
    // Construct the map file path
    let map_path = maps_dir.join(format!("{}.map", map_name));

    // Load the map file
    let map_data = std::fs::read(&map_path)
        .with_context(|| format!("Failed to load map from: {}", map_path.display()))?;

    // Verify SHA256 hash if provided in the teehistorian header
    if let Some(expected_hash) = expected_sha256 {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(&map_data);
        let actual_hash = format!("{:x}", hasher.finalize());

        if actual_hash != expected_hash {
            anyhow::bail!(
                "Map hash mismatch for {}:\n  Expected: {}\n  Actual:   {}",
                map_name,
                expected_hash,
                actual_hash
            );
        }
    }

    Ok(map_data)
}

/// Write player sequences to HDF5 file
pub fn write_hdf5(sequences: &[PlayerSequence], output_path: &Path) -> Result<()> {
    use ndarray::Array2;

    let file = hdf5::File::create(output_path)
        .with_context(|| format!("Failed to create HDF5 file: {}", output_path.display()))?;

    for (idx, seq) in sequences.iter().enumerate() {
        if seq.data.is_empty() {
            continue;
        }

        let group = file.create_group(&format!("seq_{}", idx))?;

        // Write sequential data as 2D array [ticks × 4]
        let n_ticks = seq.data.len();

        // Create a proper 2D array using ndarray
        let mut data_array = Array2::<f32>::zeros((n_ticks, 4));
        for (i, tick_data) in seq.data.iter().enumerate() {
            data_array[[i, 0]] = tick_data.pos_x;
            data_array[[i, 1]] = tick_data.pos_y;
            data_array[[i, 2]] = tick_data.aim_angle;
            data_array[[i, 3]] = tick_data.freeze_status;
        }

        let dataset = group
            .new_dataset::<f32>()
            .shape([n_ticks, 4])
            .create("data")?;
        dataset.write(&data_array)?;

        // Write metadata as attributes
        let player_name_attr = group
            .new_attr::<hdf5::types::VarLenAscii>()
            .create("player_name")?;
        player_name_attr.write_scalar(&hdf5::types::VarLenAscii::from_ascii(&seq.player_name)?)?;

        let team_attr = group.new_attr::<i32>().create("team")?;
        team_attr.write_scalar(&seq.team)?;

        // Write finish info (use -1 as sentinel for no finish)
        let finish_tick_attr = group.new_attr::<i64>().create("finish_tick")?;
        finish_tick_attr.write_scalar(&seq.finish.as_ref().map(|f| f.tick).unwrap_or(-1))?;

        let finish_duration_attr = group.new_attr::<f32>().create("finish_duration_secs")?;
        finish_duration_attr
            .write_scalar(&seq.finish.as_ref().map(|f| f.duration_secs).unwrap_or(-1.0))?;

        let start_attr = group.new_attr::<i64>().create("start_tick")?;
        start_attr.write_scalar(&seq.start_tick)?;

        let end_attr = group.new_attr::<i64>().create("end_tick")?;
        end_attr.write_scalar(&seq.end_tick)?;

        let duration_attr = group.new_attr::<i64>().create("duration_ticks")?;
        duration_attr.write_scalar(&(seq.end_tick - seq.start_tick))?;

        let map_attr = group
            .new_attr::<hdf5::types::VarLenAscii>()
            .create("map_name")?;
        map_attr.write_scalar(&hdf5::types::VarLenAscii::from_ascii(&seq.map_name)?)?;

        let time_attr = group
            .new_attr::<hdf5::types::VarLenAscii>()
            .create("time_of_day")?;
        time_attr.write_scalar(&hdf5::types::VarLenAscii::from_ascii(&seq.time_of_day)?)?;
    }

    Ok(())
}
