# teehee-extractor

**teehee-extractor** is a Rust crate that extracts sequences of player input data from DDNet teehistorian log files and writes them to HDF5 for easy interoperability across languages. Active gameplay segments are split into fixed-length sequences to ensure consistent tensor shapes suitable for machine learning pipelines. It supports parallel processing and memory-efficient batch processing to handle large collections of teehistorian files.

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

## Output Format

The tool produces two files in the output directory:

### `sequences.h5`
HDF5 dataset with shape `(num_sequences, seq_length, num_features)` containing:
- `move_dir`: movement direction (-1.0, 0.0, 1.0)
- `jump`, `fire`, `hook`: binary button states (0.0 or 1.0)
- `vel_x`, `vel_y`: Player velocity
- `aim_angle`: Cursor angle in degrees
- `aim_distance`: Distance to cursor

### `meta.csv`
Metadata for each sequence:
```
seq_id,player_id,player,start,ticks,map,teehist
...
```

## Current Limitations

- Memory usage spikes are consistent and proportional to batch size. However, there is a gradual increase in memory over time despite this, which might lead to OOM problems when processing very large collections of teehistorian files (50GB+). For now, consider increasing swap if this is an issue.
- Most relevant teehistorian chunk types are supported. Some rarely-used ones are not yet implemented.
- Sequences are extracted at fixed length to ensure uniform tensor shapes.

All of these will be addressed in future releases.
