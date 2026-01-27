use clap::Parser;
use log::info;
use log::LevelFilter;
use rayon::ThreadPoolBuilder;
use std::fs;
use std::path::PathBuf;
use teehee_extractor::export::{ExportConfig, Exporter};
use teehee_extractor::parser::ParserConfig;

#[derive(Parser, Debug)]
struct Cli {
    /// Input data directory
    #[clap(short, long, default_value = "./data/teehistorian/")]
    input: PathBuf,

    /// Filepath for output dataset folder
    #[clap(short, long, default_value = "./data/out/dataset/")]
    output_folder: PathBuf,

    /// ticks per sequence
    #[clap(short, long, default_value = "1000")]
    seq_length: usize,

    /// Ticks of no movement that counts as player being AFK
    #[clap(short, long, default_value = "500")]
    afk_ticks: usize,

    /// Ticks of padding around durations, after afk removal
    #[clap(long = "ap", default_value = "15")]
    afk_padding: usize,

    /// Logging level (error, warn, info, debug, trace)
    #[clap(short, long, default_value = "info")]
    log_level: LevelFilter,

    /// Cut sequence on player kill
    #[clap(short = 'k', long)]
    cut_kill: bool,

    /// Cut sequence on player rescue (/r)
    #[clap(short = 'r', long)]
    cut_rescue: bool,

    /// number of teehistorian files to process before saving to file
    #[clap(short = 'b', long, default_value = "1000")]
    file_chunk_size: usize,

    /// number of teehistorian files to process before saving to file
    #[clap(long, default_value = "2000")]
    max_files: usize,

    #[clap(long, default_value = "100")]
    max_speed: i32,

    #[clap(long, default_value = "1000")]
    max_aim_distance: i32,

    #[clap(short = 'd', long)]
    dry_run: bool,

    /// after export, give summary of players with top k amount of sequences
    #[clap(short = 'p', long)]
    print_top_k: Option<usize>,

    /// csv list of player names to include. All others will be filtered out.
    #[clap(short = 'f', long, value_delimiter = ',')]
    filter_players: Option<Vec<String>>,

    /// number of worker threads for parallel processing (defaults to physical CPU cores)
    #[clap(short = 'j', long)]
    workers: Option<usize>,
}

fn batched_export(args: &Cli) {
    let parser_config = ParserConfig::new(
        args.cut_kill,
        args.cut_rescue,
        args.max_speed,
        args.filter_players.clone(),
    );
    let export_config = ExportConfig {
        seq_length: args.seq_length,
        afk_ticks: args.afk_ticks,
        afk_padding: args.afk_padding,
        dry_run: args.dry_run,
        use_vel: true,
        use_rel_target: false,
        use_aim_angle: true,
        use_aim_distance: true,
    };
    let mut exporter = Exporter::new(&args.output_folder, export_config.clone());

    // get all files
    let mut paths: Vec<_> = fs::read_dir(&args.input)
        .unwrap()
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    paths.truncate(args.max_files);
    let file_count = paths.len();
    let batch_count = (file_count + args.file_chunk_size - 1) / args.file_chunk_size;
    info!("found {} files to parse", file_count);

    // process all files in batches
    for (batch_index, batch_paths) in paths.chunks(args.file_chunk_size).enumerate() {
        info!(
            "[{}/{}] parsing {} files",
            batch_index + 1,
            batch_count,
            batch_paths.len()
        );
        exporter.handle_batch(batch_paths, &parser_config, &export_config);
    }

    exporter.print_summary(args.print_top_k.unwrap_or(10));
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Cli::parse();
    dbg!(&args);
    colog::default_builder()
        .filter_level(args.log_level)
        .target(env_logger::Target::Stdout)
        .init();

    // Configure rayon thread pool
    let num_workers = args.workers.unwrap_or_else(|| num_cpus::get_physical());
    ThreadPoolBuilder::new()
        .num_threads(num_workers)
        .build_global()
        .unwrap();
    info!(
        "using {} worker threads (physical cores: {})",
        num_workers,
        num_cpus::get_physical()
    );

    batched_export(&args);
    info!("done");
    Ok(())
}
