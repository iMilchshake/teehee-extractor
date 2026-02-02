use derivative::Derivative;

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
    pub active_regions: Vec<(usize, usize)>,
    pub timeout_code: Option<String>,
}

fn fmt_vec_len<T>(v: &Vec<T>, f: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
    write!(f, "[{} items]", v.len())
}

/// Ticks of unchanged move_dir to be considered AFK (1 second at 50Hz)
const AFK_TICKS: usize = 500;

/// Remove AFK ticks (move_dir unchanged for AFK_TICKS+ consecutive ticks)
pub fn drop_afk_ticks(data: &mut Vec<TickData>) {
    let mut keep = vec![true; data.len()];
    let mut same_count = 0;

    for i in 0..data.len().saturating_sub(1) {
        if data[i + 1].move_dir == data[i].move_dir {
            same_count += 1;
            if same_count >= AFK_TICKS {
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

/// Compute active regions - splits on gaps in tick numbers
pub fn compute_active_regions(data: &[TickData]) -> Vec<(usize, usize)> {
    let mut regions = vec![];
    let mut start = 0;

    for i in 0..data.len().saturating_sub(1) {
        if data[i + 1].tick != data[i].tick + 1 {
            regions.push((start, i + 1));
            start = i + 1;
        }
    }

    if !data.is_empty() {
        regions.push((start, data.len()));
    }

    regions
}
