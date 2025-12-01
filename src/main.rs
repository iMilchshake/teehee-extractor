use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use teehee_extractor::{extract_sequences, write_hdf5};

/// Extract player sequences from DDNet teehistorian files to HDF5 format
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Input teehistorian file path
    #[arg(short, long)]
    input: PathBuf,

    /// Output HDF5 file path
    #[arg(short, long)]
    output: PathBuf,

    /// Directory containing map files (.map)
    #[arg(short, long)]
    maps_dir: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();

    println!("Extracting sequences from: {}", args.input.display());

    let sequences = extract_sequences(&args.input, &args.maps_dir)?;

    println!("Found {} player sequences", sequences.len());

    for (idx, seq) in sequences.iter().enumerate() {
        let finish_info = match &seq.finish {
            Some(f) => format!("finished at tick {} in {:.2}s", f.tick, f.duration_secs),
            None => "did not finish".to_string(),
        };
        println!(
            "  Sequence {}: {} ({} ticks, team {}, {})",
            idx,
            seq.player_name,
            seq.data.len(),
            seq.team,
            finish_info
        );
    }

    println!("Writing to HDF5: {}", args.output.display());
    write_hdf5(&sequences, &args.output)?;

    println!("Done!");

    Ok(())
}
