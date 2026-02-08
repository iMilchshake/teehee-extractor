use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use teehee_extractor::features::{Feature, FeatureSet};
use teehee_extractor::{extract_sequences, write_hdf5};

/// Extract player sequences from DDNet teehistorian files to HDF5 format
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Extract sequences from a teehistorian file
    Extract {
        /// Input teehistorian file path
        #[arg(short, long)]
        input: PathBuf,

        /// Output HDF5 file path
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
        } => {
            // parse feature selection
            let feature_set = match &features {
                Some(spec) => FeatureSet::from_spec(spec).unwrap_or_else(|e| {
                    eprintln!("Error: {e}");
                    std::process::exit(1);
                }),
                None => FeatureSet::all(),
            };

            println!("Extracting sequences from: {}", input.display());
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

            let sequences = extract_sequences(&input, &maps_dir, afk_ticks)?;

            println!("Found {} player sequences", sequences.len());

            for (idx, seq) in sequences.iter().enumerate() {
                let finish_info = if seq.finishes.is_empty() {
                    "did not finish".to_string()
                } else {
                    let times: Vec<String> = seq.finishes.iter().map(|f| format!("{:.2}s", f.duration_secs)).collect();
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
            write_hdf5(&sequences, &output, &feature_set)?;

            println!("Done!");
        }
    }

    Ok(())
}
