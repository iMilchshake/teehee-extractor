# Teehistorian Extractor

A Rust crate for extracting sequences of player input data from DDNet teehistorian log files and converting them into HDF5 datasets.

## Features

- **Parallel Processing**: Multi-threaded file parsing using rayon
- **AFK Detection**: Automatically removes idle periods where players aren't moving
- **Sequence Chunking**: Splits gameplay into fixed-length windows for consistent input shapes
- **Player Filtering**: Process only specific players by name
- **Batch Processing**: Memory-efficient processing of large replay collections
- **HDF5 Export**: Scientific data format with CSV metadata for easy loading


## Basic Usage

```bash
# Process all .teehistorian files in a directory
cargo run --release -- -i ./teehistorian/ -o ./dataset/
```

## Command Line Options

| Flag | Description | Default |
|------|-------------|---------|
| `-i, --input` | Input directory containing teehistorian files | `./data/teehistorian/` |
| `-o, --output-folder` | Output directory for dataset | `./data/out/dataset/` |
| `-s, --seq-length` | Ticks per sequence | `1000` |
| `-a, --afk-ticks` | Ticks of no movement to classify as AFK | `500` |
| `--ap` | Padding ticks around non-AFK periods | `15` |
| `-k, --cut-kill` | End sequence when player dies | `false` |
| `-r, --cut-rescue` | End sequence on /r (rescue) command | `false` |
| `--max-speed` | Max pixels/tick before detecting teleport | `100` |
| `--max-aim-distance` | Cap aim distance at this value | `1000` |
| `-b, --file-chunk-size` | Files per batch before writing to disk | `1000` |
| `--max-files` | Maximum files to process | `2000` |
| `-j, --workers` | Thread count (defaults to physical cores) | auto |
| `-f, --filter-players` | Comma-separated list of player names to include | all |
| `-p, --print-top-k` | Print top K players by sequence count | `10` |
| `-d, --dry-run` | Parse files without writing output | `false` |
| `-l, --log-level` | Logging verbosity (error/warn/info/debug/trace) | `info` |

### Examples

```bash
# Process with 8 threads, 2000-tick sequences
cargo run --release -- -i ./replays/ -o ./out/ -s 2000 -j 8

# Only extract data for specific players
cargo run --release -- -i ./replays/ -o ./out/ -f "Player1,Player2,Player3"

# Cut sequences on death, useful for training death-prediction models
cargo run --release -- -i ./replays/ -o ./out/ -k

# Dry run to see statistics without writing files
cargo run --release -- -i ./replays/ -d -p 20
```

## Output Format

The tool produces two files in the output directory:

### `sequences.h5`
HDF5 dataset with shape `(num_sequences, seq_length, num_features)` containing:
- `move_dir`: Ternary movement direction (-1, 0, 1)
- `jump`, `fire`, `hook`: Binary button states (0.0 or 1.0)
- `vel_x`, `vel_y`: Player velocity 
- `aim_angle`: Cursor angle in degrees
- `aim_distance`: Distance to cursor

### `meta.csv`
Metadata for each sequence:
```
seq_id,player_id,player,start,ticks,map,teehist
...
```
