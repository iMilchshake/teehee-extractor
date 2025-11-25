use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs::File;
use std::path::Path;
use std::sync::Arc;
use teehistorian_replayer::teehistorian::{ThBufReader, ThCompat, ThStream};
use teehistorian_replayer::twgame_core::replay::{DemoChatWrite, DemoWrite};
use teehistorian_replayer::twgame_core::twsnap::compat::ddnet::WriteError;
use teehistorian_replayer::twgame_core::twsnap::time::Instant;
use teehistorian_replayer::twgame_core::twsnap::Snap;
use teehistorian_replayer::ThReplayer;
use twgame::{DdnetReplayerWorld, Map};

/// Data recorded for each game tick
#[derive(Debug, Clone)]
pub struct TickData {
    pub tick: i64,
    pub pos_x: f32,
    pub pos_y: f32,
    pub aim_angle: f32,
    pub freeze_status: f32,
}

/// Complete sequence of a player from join to leave
#[derive(Debug, Clone)]
pub struct PlayerSequence {
    pub player_name: String,
    pub team: i32,
    pub finished: bool,
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
}

impl DataCapturingWriter {
    fn new() -> Self {
        Self {
            sequences: HashMap::new(),
            player_info: HashMap::new(),
            current_tick: 0,
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

// Implement DemoWrite to capture game state at each tick
impl DemoWrite<DdnetReplayerWorld> for DataCapturingWriter {
    fn snap_and_write(
        &mut self,
        tick: Instant,
        _world: &DdnetReplayerWorld,
        snap_buf: &mut Snap,
    ) -> Result<(), WriteError> {
        // Convert Instant to tick number
        self.current_tick = tick.snap_tick() as i64;

        // Extract player/tee data from the snapshot
        // The snapshot contains all game state including positions, angles, and freeze status
        for (snap_id, player) in snap_buf.players.iter() {
            // Store player info (name and team)
            self.player_info
                .entry(snap_id.0)
                .or_insert_with(|| (player.name.to_string(), player.team));

            if let Some(tee) = &player.tee {
                // Extract aim angle (already in radians)
                let aim_angle = tee.angle.to_num::<f32>();

                // Determine freeze status (frozen if freeze_end > current tick)
                let is_frozen = tee.freeze_end > tick;
                let freeze_status = if is_frozen { 1.0 } else { 0.0 };

                let tick_data = TickData {
                    tick: self.current_tick,
                    pos_x: tee.pos.x.to_num::<f32>(),
                    pos_y: tee.pos.y.to_num::<f32>(),
                    aim_angle,
                    freeze_status,
                };

                // Use snap_id as player identifier
                self.sequences
                    .entry(snap_id.0)
                    .or_insert_with(Vec::new)
                    .push(tick_data);
            }
        }

        Ok(())
    }

    fn chat(&mut self) -> &mut (dyn DemoChatWrite + 'static) {
        self
    }
}

/// Extract player sequences from a teehistorian file using the replayer
pub fn extract_sequences(teehistorian_path: &Path) -> Result<Vec<PlayerSequence>> {
    let file = File::open(teehistorian_path)
        .with_context(|| format!("Failed to open file: {}", teehistorian_path.display()))?;

    let mut th_stream = ThCompat::parse(ThBufReader::new(file))?;

    // Extract map name from header
    let mut map_name = String::from("unknown");
    if let Ok(header) = th_stream.header() {
        if let Ok(header_str) = std::str::from_utf8(header) {
            if let Some(map_start) = header_str.find("map_name") {
                if let Some(map_slice) = header_str.get(map_start..) {
                    if let Some(name) = map_slice.split('\0').nth(1) {
                        map_name = name.to_string();
                    }
                }
            }
        }
    }

    // Create a minimal map for the world
    // In production, load the actual map file from the teehistorian header
    let map_data = create_minimal_map()?;
    let mut parsed_map = twmap::TwMap::parse(&map_data)?;
    let map = Map::try_from(&mut parsed_map).map_err(|e| anyhow::anyhow!(e))?;
    let map = Arc::new(map);

    // Create the replayer world
    let header_raw = th_stream.header()?;
    let mut world = DdnetReplayerWorld::new(map, false);

    // Create our custom data capturing writer
    let mut data_writer = DataCapturingWriter::new();

    // Use the replayer to replay the game and capture data
    let replayer = ThReplayer::new(header_raw, &mut world);
    replayer.validate(&mut world, &mut th_stream, Some(&mut data_writer));

    // Convert captured data to PlayerSequence
    let time_of_day = chrono::Utc::now().to_rfc3339(); // TODO: now? xd
    let sequences: Vec<PlayerSequence> = data_writer
        .sequences
        .into_iter()
        .map(|(player_id, data)| {
            let start_tick = data.first().map(|d| d.tick).unwrap_or(0);
            let end_tick = data.last().map(|d| d.tick).unwrap_or(0);

            // Get player info (name and team)
            let (player_name, team) = data_writer
                .player_info
                .get(&player_id)
                .cloned()
                .unwrap_or_else(|| (format!("player_{}", player_id), 0));

            PlayerSequence {
                player_name,
                team,
                finished: false, // TODO: could extract from finishes in world
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

/// Create a minimal valid DDNet map
///
/// NOTE: This creates a very basic empty map. For production use with real
/// teehistorian files, you should load the actual map file referenced in the header.
fn create_minimal_map() -> Result<Vec<u8>> {
    // Use a pre-generated minimal valid map binary
    // This is a tiny valid DDNet map with no tiles
    // In production, load the actual map file from disk based on the teehistorian header

    // For now, return an error directing users to provide the map
    // TODO: Either embed a minimal map binary or generate one properly
    Err(anyhow::anyhow!(
        "Map creation not yet implemented. Please provide the actual map file.\n\
         You can load maps using the map_name from the teehistorian header."
    ))

    // Alternative: If users have a minimal.map file, they could load it:
    // std::fs::read("path/to/minimal.map").context("Failed to load minimal map")
}

/// Write player sequences to HDF5 file
pub fn write_hdf5(sequences: &[PlayerSequence], output_path: &Path) -> Result<()> {
    let file = hdf5::File::create(output_path)
        .with_context(|| format!("Failed to create HDF5 file: {}", output_path.display()))?;

    for (idx, seq) in sequences.iter().enumerate() {
        if seq.data.is_empty() {
            continue;
        }

        let group = file.create_group(&format!("seq_{}", idx))?;

        // Write sequential data as 2D array [ticks × 4]
        let n_ticks = seq.data.len();
        let mut data_array = vec![0.0f32; n_ticks * 4];

        for (i, tick_data) in seq.data.iter().enumerate() {
            data_array[i * 4 + 0] = tick_data.pos_x;
            data_array[i * 4 + 1] = tick_data.pos_y;
            data_array[i * 4 + 2] = tick_data.aim_angle;
            data_array[i * 4 + 3] = tick_data.freeze_status;
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

        let finished_attr = group.new_attr::<i32>().create("finished")?;
        finished_attr.write_scalar(&(seq.finished as i32))?;

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
