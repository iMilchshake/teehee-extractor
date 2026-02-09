use anyhow::Result;
use clap::{Parser, Subcommand};
use mimalloc::MiMalloc;
use rayon::prelude::*;
use std::path::PathBuf;
use std::time::Instant;
use teehee_extractor::features::{Feature, FeatureSet};
use teehee_extractor::{extract_sequences, write_hdf5, ParquetWriter};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

/// Extract player sequences from DDNet teehistorian files to HDF5 format
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Extract sequences from a teehistorian file or directory of files
    Extract {
        /// Input teehistorian file or directory path
        #[arg(short, long)]
        input: PathBuf,

        /// Output path (HDF5 file for single file input, directory of parquet files for directory input)
        #[arg(short, long)]
        output: PathBuf,

        /// Directory containing map files (.map)
        #[arg(short, long)]
        maps_dir: PathBuf,

        /// Features to extract (comma-separated). Can use group names or individual features.
        /// Groups: position, velocity, cursor, aim, movement, inputs, state, hook, weapons, jumps
        /// Default: all features
        #[arg(short, long)]
        features: Option<String>,

        /// AFK threshold in seconds. Ticks where move_dir is unchanged for longer
        /// than this are removed. Default: 10
        #[arg(long, default_value = "10")]
        afk: f32,

        /// Disable AFK removal entirely
        #[arg(long)]
        no_afk: bool,

        /// Number of parallel threads for directory processing (default: all cores)
        #[arg(long)]
        threads: Option<usize>,

        /// Only process the first N files (useful for testing)
        #[arg(long)]
        limit: Option<usize>,
    },
    /// List all available features and groups
    ListFeatures,
}

fn main() -> Result<()> {
    let args = Args::parse();

    match args.command {
        Command::ListFeatures => {
            println!("Available feature groups:");
            for group in Feature::groups() {
                let features: Vec<_> = Feature::in_group(group).iter().map(|f| f.name()).collect();
                println!("  {}: {}", group, features.join(", "));
            }
            println!(
                "\nAll features: {}",
                Feature::all()
                    .iter()
                    .map(|f| f.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        Command::Extract {
            input,
            output,
            maps_dir,
            features,
            afk,
            no_afk,
            threads,
            limit,
        } => {
            // parse feature selection
            let feature_set = match &features {
                Some(spec) => FeatureSet::from_spec(spec).unwrap_or_else(|e| {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }),
                None => FeatureSet::all(),
            };

            println!(
                "Selected features ({}): {}",
                feature_set.len(),
                feature_set.names().join(", ")
            );

            let afk_ticks = if no_afk {
                None
            } else {
                Some((afk * 50.0) as usize)
            };

            if let Some(n) = threads {
                rayon::ThreadPoolBuilder::new()
                    .num_threads(n)
                    .build_global()
                    .expect("failed to set thread count");
            }

            if input.is_dir() {
                extract_directory(&input, &output, &maps_dir, &feature_set, afk_ticks, limit)?;
            } else {
                extract_single_file(&input, &output, &maps_dir, &feature_set, afk_ticks)?;
            }
        }
    }

    Ok(())
}

fn extract_single_file(
    input: &PathBuf,
    output: &PathBuf,
    maps_dir: &PathBuf,
    feature_set: &FeatureSet,
    afk_ticks: Option<usize>,
) -> Result<()> {
    println!("Extracting sequences from: {}", input.display());

    let sequences = extract_sequences(input, maps_dir, afk_ticks)?;

    println!("Found {} player sequences", sequences.len());

    for (idx, seq) in sequences.iter().enumerate() {
        let finish_info = if seq.finishes.is_empty() {
            "did not finish".to_string()
        } else {
            let times: Vec<String> = seq
                .finishes
                .iter()
                .map(|f| format!("{:.2}s", f.duration_secs))
                .collect();
            format!("{} finish(es): {}", seq.finishes.len(), times.join(", "))
        };
        println!(
            "  Sequence {}: {} ({} ticks, team {}, {})",
            idx,
            seq.player_name,
            seq.data.len(),
            seq.team,
            finish_info
        );
        dbg!(&seq);
    }

    println!("Writing to HDF5: {}", output.display());
    write_hdf5(&sequences, output, feature_set)?;

    println!("Done!");
    Ok(())
}

fn extract_directory(
    input_dir: &PathBuf,
    output_dir: &PathBuf,
    maps_dir: &PathBuf,
    feature_set: &FeatureSet,
    afk_ticks: Option<usize>,
    limit: Option<usize>,
) -> Result<()> {
    // collect all .teehistorian files
    let mut files: Vec<PathBuf> = std::fs::read_dir(input_dir)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|ext| ext == "teehistorian") {
                Some(path)
            } else {
                None
            }
        })
        .collect();
    files.sort();
    if let Some(n) = limit {
        files.truncate(n);
    }

    let n_files = files.len();
    println!(
        "Found {n_files} teehistorian files in {}",
        input_dir.display()
    );

    if n_files == 0 {
        println!("Nothing to do.");
        return Ok(());
    }

    std::fs::create_dir_all(output_dir)?;

    let started = Instant::now();
    let errors = std::sync::atomic::AtomicUsize::new(0);
    let processed = std::sync::atomic::AtomicUsize::new(0);
    let total_sequences = std::sync::atomic::AtomicUsize::new(0);

    // each rayon worker extracts and writes its own parquet file — fully parallel
    files.par_iter().for_each(|file| {
        let filename = file
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        // catch panics (e.g. assert failures on corrupt data)
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            extract_and_write_parquet(file, maps_dir, output_dir, feature_set, afk_ticks)
        }));

        let n = processed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;

        match result {
            Ok(Ok((n_seq, n_ticks))) => {
                if n_seq > 0 {
                    total_sequences.fetch_add(n_seq, std::sync::atomic::Ordering::Relaxed);
                    eprintln!("[{n}/{n_files}] {filename} -> {n_seq} sequences, {n_ticks} ticks");
                } else {
                    eprintln!("[{n}/{n_files}] {filename} -> empty, skipped");
                }
            }
            Ok(Err(e)) => {
                errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                eprintln!("[{n}/{n_files}] {filename} -> ERROR: {e}");
            }
            Err(_panic) => {
                errors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                eprintln!("[{n}/{n_files}] {filename} -> PANIC (corrupt data?)");
            }
        }
    });

    let elapsed = started.elapsed();
    let n_errors = errors.load(std::sync::atomic::Ordering::Relaxed);
    let n_seqs = total_sequences.load(std::sync::atomic::Ordering::Relaxed);

    println!(
        "Done! Wrote {n_seqs} sequences to {} in {:.1}s ({n_files} files, {n_errors} errors)",
        output_dir.display(),
        elapsed.as_secs_f64()
    );

    Ok(())
}

/// Extract sequences from a single teehistorian file and write to a parquet file
/// in the output directory. Returns (n_sequences, n_ticks).
fn extract_and_write_parquet(
    file: &PathBuf,
    maps_dir: &PathBuf,
    output_dir: &PathBuf,
    feature_set: &FeatureSet,
    afk_ticks: Option<usize>,
) -> Result<(usize, usize)> {
    let sequences = extract_sequences(file, maps_dir, afk_ticks)?;
    if sequences.is_empty() {
        return Ok((0, 0));
    }

    let n_seq = sequences.len();
    let n_ticks: usize = sequences.iter().map(|s| s.data.len()).sum();

    let out_name = file.file_stem().unwrap_or_default().to_string_lossy();
    let out_path = output_dir.join(format!("{out_name}.parquet"));

    let mut writer = ParquetWriter::create(&out_path, feature_set.clone())?;
    for seq in &sequences {
        writer.write_sequence(seq)?;
    }
    writer.finish()?;

    Ok((n_seq, n_ticks))
}
