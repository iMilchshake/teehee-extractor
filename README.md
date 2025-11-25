# Teehee Extractor

Extract player input data and game state from DDNet teehistorian recordings into HDF5 format for machine learning applications.

## Usage

```bash
# Build the project
cargo build --release

# Run the extractor
cargo run --release -- --input <teehistorian_file> --output <output.h5>
```

## Example

```bash
cargo run --release -- --input recording.teehistorian --output data.h5
```

## Output Format

The HDF5 file contains one dataset per player sequence (from join to leave):

```
/seq_0/data: [N_ticks × 4] array
  - Column 0: pos_x (f32)
  - Column 1: pos_y (f32)
  - Column 2: aim_angle (f32, radians)
  - Column 3: freeze_status (f32, 0=not frozen, 1=frozen)

/seq_0/data attributes:
  - player_name: string
  - team: i32 (0=solo, >0=team number)
  - finished: i32 (0=did not finish, 1=finished map)
  - start_tick: i64
  - end_tick: i64
  - duration_ticks: i64
  - map_name: string
  - time_of_day: string (ISO 8601 timestamp)
```

## Data Rate

Data is recorded at every game tick (50 Hz / 20ms per tick).

## Dependencies

- teehistorian-replayer: Parse DDNet teehistorian format
- hdf5: Write output in HDF5 format
- clap: Command-line interface
