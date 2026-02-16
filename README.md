# Teehee Extractor

Extract player input data and game state from DDNet teehistorian recordings into Parquet format for machine learning applications.

## Requirements

- Rust toolchain (edition 2021)
- DDNet map files (`.map`) for the recordings you want to process

## Usage

```bash
cargo build --release

# extract a directory of recordings → Parquet (one file per input, parallel)
cargo run --release -- extract -i /path/to/recordings/ -o /path/to/output/ -m /path/to/maps/

# select specific features
cargo run --release -- extract -i recs/ -o out/ -m maps/ -f "position,velocity,inputs"

# list available features and groups
cargo run --release -- list-features
```

### Options

| Flag | Description |
|---|---|
| `-i, --input` | Input teehistorian file or directory |
| `-o, --output` | Output directory for Parquet files |
| `-m, --maps-dir` | Directory containing `.map` files |
| `-f, --features` | Comma-separated features or groups (default: all) |
| `--afk` | AFK threshold in seconds (default: 10) |
| `--no-afk` | Disable AFK removal |
| `--threads` | Number of parallel threads (default: all cores) |
| `--limit` | Only process the first N files |

### Logging

Default log level is `info`. Set `RUST_LOG=teehee_extractor=debug` for verbose output.

## Features (22)

| Group | Features |
|---|---|
| position | pos_x, pos_y |
| velocity | vel_x, vel_y |
| cursor | cursor_x, cursor_y |
| aim | aim_angle, aim_distance |
| movement | move_dir |
| inputs | key_jump, key_fire, key_hook |
| state | is_grounded, freeze_status |
| hook | hook_grabbed, hook_pos_x, hook_pos_y |
| weapons | is_hammer, is_gun, is_other_weapon |
| jumps | jumps_remaining, can_jump |

## Output Format

One `.parquet` file per input recording (ZSTD compressed). Each row is one game tick (50 Hz) for one player. Players with less than 10 seconds of data are dropped.

### Metadata columns

| Column | Type | Description |
|---|---|---|
| `seq_id` | uint32 | Sequence index within the file (one sequence = one player session from join to leave) |
| `player_name` | string | Player name (from snap data) |
| `team` | int32 | Team number (0 = solo) |
| `map_name` | string | Map being played |
| `time_of_day` | string | ISO 8601 timestamp of the recording |
| `timeout_code` | string | Timeout reconnect code (empty if none) |
| `tick` | int64 | Game tick number |

### Region columns

A sequence is split into contiguous regions whenever gameplay is interrupted (teleport, kill, team change, spectate, AFK, etc.). Each tick belongs to exactly one region.

| Column | Type | Description |
|---|---|---|
| `region_id` | uint32 | Region index within the sequence |
| `region_team` | int32 | Team number during this region |
| `region_practice` | bool | Whether practice mode (or sv_rescue) was active |
| `region_end_reason` | uint8 | Why this region ended (see table below) |

#### Region end reasons

| Value | Name | Description |
|---|---|---|
| 0 | Kill | Player used `/kill` |
| 1 | TeamChange | Player changed team |
| 2 | SwapTees | Tees swapped between players |
| 3 | Spectate | Gap from spectating (detected post-hoc) |
| 4 | Afk | Gap from AFK removal (detected post-hoc) |
| 5 | Leave | Player left the server |
| 6 | ChangeMap | Recording ended (map change or server shutdown) |
| 7 | Unknown | Gap with unknown cause |
| 8 | Teleport | Teleporter tile, practice `/r`, or sv_rescue |
| 9 | UnknownTeleport | Large position jump with no known cause |
| 10 | TeeDisappear | Tee removed from world (death/kill tile) |

The mapping is also stored in the Parquet file metadata under the key `end_reason_names`.

### Feature columns

All feature columns are `float32`. Which features are included depends on the `-f` flag (default: all 22).

| Column | Unit / range | Description |
|---|---|---|
| `pos_x`, `pos_y` | tiles | World position |
| `vel_x`, `vel_y` | tiles/tick | Velocity |
| `cursor_x`, `cursor_y` | raw | Aim target position (client-reported) |
| `aim_angle` | radians | Aim direction (-pi to pi) |
| `aim_distance` | raw | Distance to aim target |
| `move_dir` | -1, 0, 1 | Horizontal movement input |
| `key_jump` | 0 or 1 | Jump key pressed |
| `key_fire` | 0 or 1 | Fire key pressed |
| `key_hook` | 0 or 1 | Hook key pressed |
| `is_grounded` | 0 or 1 | Whether tee is on ground |
| `freeze_status` | 0 or 1 | Whether tee is frozen |
| `hook_grabbed` | 0 or 1 | Whether hook is attached |
| `hook_pos_x`, `hook_pos_y` | tiles | Hook endpoint position |
| `is_hammer` | 0 or 1 | Hammer selected |
| `is_gun` | 0 or 1 | Gun selected |
| `is_other_weapon` | 0 or 1 | Other weapon selected (shotgun, grenade, laser) |
| `jumps_remaining` | 0-3 | Available jumps |
| `can_jump` | 0 or 1 | Whether tee can currently jump |

## Analysis

```bash
python scripts/stats.py /path/to/output/
```

Prints summary statistics (sequence counts, feature ranges, region end reasons, disk usage) over exported Parquet data. Requires `polars`.
