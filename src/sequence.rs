use derivative::Derivative;

/// Reason why an active region ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionEndReason {
    /// Player used /kill command
    Kill,
    /// Player changed team via /team command
    TeamChange,
    /// Tees were swapped between players
    SwapTees,
    /// Tick gap from spectating (detected post-hoc)
    Spectate,
    /// Tick gap from AFK removal (detected post-hoc)
    Afk,
    /// Player left the server
    Leave,
    /// Teehistorian file ended (map change or server shutdown)
    ChangeMap,
    /// Tick gap with unknown cause
    Unknown,
}

impl RegionEndReason {
    /// Convert to u8 for HDF5 storage
    pub fn as_u8(&self) -> u8 {
        match self {
            RegionEndReason::Kill => 0,
            RegionEndReason::TeamChange => 1,
            RegionEndReason::SwapTees => 2,
            RegionEndReason::Spectate => 3,
            RegionEndReason::Afk => 4,
            RegionEndReason::Leave => 5,
            RegionEndReason::ChangeMap => 6,
            RegionEndReason::Unknown => 7,
        }
    }

    /// Names for all enum variants (for HDF5 attribute)
    pub fn names() -> &'static str {
        "Kill,TeamChange,SwapTees,Spectate,Afk,Leave,ChangeMap,Unknown"
    }
}

/// An active region of gameplay with metadata.
#[derive(Debug, Clone)]
pub struct ActiveRegion {
    /// Start tick of this region
    pub start_tick: i64,
    /// End tick of this region
    pub end_tick: i64,
    /// Team number during this region
    pub team: i32,
    /// Whether practice mode was enabled during this region
    pub practice: bool,
    /// Reason why this region ended
    pub end_reason: RegionEndReason,
    /// Start index in data array (computed after AFK removal)
    pub start_idx: Option<usize>,
    /// End index in data array (computed after AFK removal)
    pub end_idx: Option<usize>,
}

impl ActiveRegion {
    /// Create a new active region
    pub fn new(
        start_tick: i64,
        end_tick: i64,
        team: i32,
        practice: bool,
        end_reason: RegionEndReason,
    ) -> Self {
        Self {
            start_tick,
            end_tick,
            team,
            practice,
            end_reason,
            start_idx: None,
            end_idx: None,
        }
    }
}

/// Data recorded for each game tick for one player.
#[derive(Debug, Clone)]
pub struct TickData {
    pub tick: i64,
    // position
    pub pos_x: f32,
    pub pos_y: f32,
    // velocity
    pub vel_x: f32,
    pub vel_y: f32,
    // cursor position (raw coordinates)
    pub cursor_x: f32,
    pub cursor_y: f32,
    // aim (polar coordinates)
    pub aim_angle: f32,
    pub aim_distance: f32,
    // movement direction (-1., 0., 1.)
    pub move_dir: f32,
    // input keys (bool flags)
    pub key_jump: f32,
    pub key_fire: f32,
    pub key_hook: f32,
    // game state
    pub is_grounded: f32,
    pub freeze_status: f32,
    // hook
    pub hook_grabbed: f32,
    pub hook_pos_x: f32,
    pub hook_pos_y: f32,
    // weapon selection (bool flags)
    pub is_hammer: f32,
    pub is_gun: f32,
    pub is_other_weapon: f32,
    // jumping
    pub jumps_remaining: f32,
    pub can_jump: f32,
}

/// Information about when and how a player finished the map.
#[derive(Debug, Clone)]
pub struct FinishInfo {
    /// Game tick when finish occurred.
    pub tick: i64,
    /// Run duration in seconds.
    pub duration_secs: f32,
}

// TODO: rename into smth more meaningful (this stores one session between joining and leaving the
// server on one map, for one player).
/// Stores all data for a complete player sequence (join to leave).
#[derive(Derivative, Clone)]
#[derivative(Debug)]
pub struct PlayerSequence {
    pub player_name: String,
    pub team: i32,
    pub finishes: Vec<FinishInfo>,
    pub start_tick: i64,
    pub end_tick: i64,
    pub time_of_day: String,
    pub map_name: String,
    #[derivative(Debug(format_with = "fmt_vec_len"))]
    pub data: Vec<TickData>,
    pub active_regions: Vec<ActiveRegion>,
    pub timeout_code: Option<String>,
}

// TODO: move to utils
fn fmt_vec_len<T>(v: &Vec<T>, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
    write!(f, "[{} items]", v.len())
}

/// Remove AFK ticks (move_dir unchanged for `afk_ticks`+ consecutive ticks)
pub fn drop_afk_ticks(data: &mut Vec<TickData>, afk_ticks: usize) {
    let mut keep = vec![true; data.len()];
    let mut same_count = 0;

    for i in 0..data.len().saturating_sub(1) {
        if data[i + 1].move_dir == data[i].move_dir {
            same_count += 1;
            if same_count >= afk_ticks {
                keep[i + 1] = false;
            }
        } else {
            same_count = 0;
        }
    }

    let mut i = 0;
    data.retain(|_| {
        let k = keep[i];
        i += 1;
        k
    });
}

/// Find all tick gaps in data (returns pairs of (gap_start_tick, gap_end_tick))
fn find_tick_gaps(data: &[TickData]) -> Vec<(i64, i64)> {
    let mut gaps = vec![];
    for i in 0..data.len().saturating_sub(1) {
        if data[i + 1].tick != data[i].tick + 1 {
            gaps.push((data[i].tick, data[i + 1].tick));
        }
    }
    gaps
}

/// Resolve tick-based region boundaries to data array indices.
/// Call this after AFK removal to compute start_idx and end_idx for each region.
pub fn resolve_region_indices(regions: &mut [ActiveRegion], data: &[TickData]) {
    if data.is_empty() {
        return;
    }

    for region in regions.iter_mut() {
        // find start index: first tick >= start_tick AND <= end_tick
        region.start_idx = data
            .iter()
            .position(|t| t.tick >= region.start_tick && t.tick <= region.end_tick);

        // find end index: last tick within region bounds (exclusive end)
        region.end_idx = data
            .iter()
            .rposition(|t| t.tick >= region.start_tick && t.tick <= region.end_tick)
            .map(|idx| idx + 1);
    }
}

/// Split existing regions on tick gaps found in data.
/// Used for post-hoc detection of Spectate and AFK gaps.
pub fn split_regions_on_gaps(
    regions: Vec<ActiveRegion>,
    data: &[TickData],
    end_reason: RegionEndReason,
) -> Vec<ActiveRegion> {
    let gaps = find_tick_gaps(data);
    if gaps.is_empty() {
        return regions;
    }

    let mut result = vec![];

    for region in regions {
        // find gaps that start within this region
        let region_gaps: Vec<_> = gaps
            .iter()
            .filter(|(gap_start, _gap_end)| {
                *gap_start >= region.start_tick && *gap_start < region.end_tick
            })
            .collect();

        if region_gaps.is_empty() {
            result.push(region);
        } else {
            // split the region on each gap
            let mut current_start = region.start_tick;

            for (gap_start, gap_end) in region_gaps {
                // region before the gap
                result.push(ActiveRegion::new(
                    current_start,
                    *gap_start,
                    region.team,
                    region.practice,
                    end_reason,
                ));
                current_start = *gap_end;
            }

            // final region after last gap (keeps original end_reason)
            // skip if the last gap extended past the region boundary
            if current_start <= region.end_tick {
                result.push(ActiveRegion::new(
                    current_start,
                    region.end_tick,
                    region.team,
                    region.practice,
                    region.end_reason,
                ));
            }
        }
    }

    result
}

/// Compute active regions - splits on gaps in tick numbers (legacy function for compatibility)
#[allow(dead_code)]
pub fn compute_active_regions(data: &[TickData]) -> Vec<ActiveRegion> {
    let mut regions = vec![];
    let mut start_idx = 0;

    for i in 0..data.len().saturating_sub(1) {
        if data[i + 1].tick != data[i].tick + 1 {
            regions.push(ActiveRegion {
                start_tick: data[start_idx].tick,
                end_tick: data[i].tick,
                team: 0,
                practice: false,
                end_reason: RegionEndReason::Unknown,
                start_idx: Some(start_idx),
                end_idx: Some(i + 1),
            });
            start_idx = i + 1;
        }
    }

    if !data.is_empty() {
        regions.push(ActiveRegion {
            start_tick: data[start_idx].tick,
            end_tick: data[data.len() - 1].tick,
            team: 0,
            practice: false,
            end_reason: RegionEndReason::Unknown,
            start_idx: Some(start_idx),
            end_idx: Some(data.len()),
        });
    }

    regions
}
