use crate::features::FeatureSet;
use crate::sequence::PlayerSequence;
use anyhow::{Context, Result};
use ndarray::Array2;
use std::path::Path;

/// Write player sequences to HDF5 file with selected features.
pub fn write_hdf5(
    sequences: &[PlayerSequence],
    output_path: &Path,
    feature_set: &FeatureSet,
) -> Result<()> {
    let file = hdf5::File::create(output_path)
        .with_context(|| format!("Failed to create HDF5 file: {}", output_path.display()))?;

    // store feature names as root-level attribute for consumers to know column ordering
    let feature_names = feature_set.names().join(",");
    let features_attr = file
        .new_attr::<hdf5::types::VarLenAscii>()
        .create("feature_names")?;
    features_attr.write_scalar(&hdf5::types::VarLenAscii::from_ascii(&feature_names)?)?;

    let n_features = feature_set.len();

    for (idx, seq) in sequences.iter().enumerate() {
        if seq.data.is_empty() {
            continue;
        }

        let group = file.create_group(&format!("seq_{idx}"))?;

        // convert Vec<TickData> to 2D array with [ticks x n_features]
        let n_ticks = seq.data.len();
        let mut data_array = Array2::<f32>::zeros((n_ticks, n_features));
        for (i, tick) in seq.data.iter().enumerate() {
            let values = feature_set.extract(tick);
            for (j, val) in values.into_iter().enumerate() {
                data_array[[i, j]] = val;
            }
        }
        let dataset = group
            .new_dataset::<f32>()
            .shape([n_ticks, n_features])
            .create("data")?;
        dataset.write(&data_array)?;

        // write metadata: player name, team, map name
        let player_name_attr = group
            .new_attr::<hdf5::types::VarLenUnicode>()
            .create("player_name")?;
        player_name_attr.write_scalar(
            &seq.player_name
                .parse::<hdf5::types::VarLenUnicode>()
                .unwrap(),
        )?;
        let team_attr = group.new_attr::<i32>().create("team")?;
        team_attr.write_scalar(&seq.team)?;
        let map_attr = group
            .new_attr::<hdf5::types::VarLenAscii>()
            .create("map_name")?;
        map_attr.write_scalar(&hdf5::types::VarLenAscii::from_ascii(&seq.map_name)?)?;

        // write finishes as [n_finishes x 2] dataset (tick, duration_secs)
        let n_finishes = seq.finishes.len();
        let finishes_dataset = group
            .new_dataset::<f64>()
            .shape([n_finishes, 2])
            .create("finishes")?;
        let finishes_data: Vec<f64> = seq
            .finishes
            .iter()
            .flat_map(|f| [f.tick as f64, f.duration_secs as f64])
            .collect();
        finishes_dataset.write_raw(&finishes_data)?;

        // write metadata: tick and time info
        let start_attr = group.new_attr::<i64>().create("start_tick")?;
        start_attr.write_scalar(&seq.start_tick)?;
        let end_attr = group.new_attr::<i64>().create("end_tick")?;
        end_attr.write_scalar(&seq.end_tick)?;
        let duration_attr = group.new_attr::<i64>().create("duration_ticks")?;
        duration_attr.write_scalar(&(seq.end_tick - seq.start_tick))?;
        let time_attr = group
            .new_attr::<hdf5::types::VarLenAscii>()
            .create("time_of_day")?;
        time_attr.write_scalar(&hdf5::types::VarLenAscii::from_ascii(&seq.time_of_day)?)?;

        // write active_regions as [n_regions x 2] dataset
        let n_regions = seq.active_regions.len();
        let regions_dataset = group
            .new_dataset::<u64>()
            .shape([n_regions, 2])
            .create("active_regions")?;
        let regions_data: Vec<u64> = seq
            .active_regions
            .iter()
            .flat_map(|(start, end)| [*start as u64, *end as u64])
            .collect();
        regions_dataset.write_raw(&regions_data)?;

        // write timeout_code if present
        if let Some(ref timeout_code) = seq.timeout_code {
            let timeout_attr = group
                .new_attr::<hdf5::types::VarLenAscii>()
                .create("timeout_code")?;
            timeout_attr.write_scalar(&hdf5::types::VarLenAscii::from_ascii(timeout_code)?)?;
        }
    }

    Ok(())
}
