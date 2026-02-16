use crate::sequence::PlayerSequence;

/// Cap cursor distance while preserving aim angle.
/// Rescales cursor_x/cursor_y so that aim_distance <= max_dist,
/// keeping the direction vector intact.
pub fn cap_cursor_distance(sequences: &mut [PlayerSequence], max_dist: f32) {
    for seq in sequences.iter_mut() {
        for tick in seq.data.iter_mut() {
            if tick.aim_distance > max_dist {
                let scale = max_dist / tick.aim_distance;
                tick.cursor_x *= scale;
                tick.cursor_y *= scale;
                tick.aim_distance = max_dist;
            }
        }
    }
}
