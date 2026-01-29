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
