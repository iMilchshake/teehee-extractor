use crate::features::FeatureSet;
use crate::sequence::{PlayerSequence, RegionEndReason};
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

        // write active_regions as a nested group with multiple datasets
        let n_regions = seq.active_regions.len();
        let regions_group = group.create_group("active_regions")?;

        // ticks: [n_regions x 2] i64 (start_tick, end_tick)
        let ticks_dataset = regions_group
            .new_dataset::<i64>()
            .shape([n_regions, 2])
            .create("ticks")?;
        let ticks_data: Vec<i64> = seq
            .active_regions
            .iter()
            .flat_map(|r| [r.start_tick, r.end_tick])
            .collect();
        ticks_dataset.write_raw(&ticks_data)?;

        // indices: [n_regions x 2] u64 (start_idx, end_idx)
        let indices_dataset = regions_group
            .new_dataset::<u64>()
            .shape([n_regions, 2])
            .create("indices")?;
        let indices_data: Vec<u64> = seq
            .active_regions
            .iter()
            .flat_map(|r| {
                [
                    r.start_idx.unwrap_or(0) as u64,
                    r.end_idx.unwrap_or(0) as u64,
                ]
            })
            .collect();
        indices_dataset.write_raw(&indices_data)?;

        // team: [n_regions] i32
        let team_dataset = regions_group
            .new_dataset::<i32>()
            .shape([n_regions])
            .create("team")?;
        let team_data: Vec<i32> = seq.active_regions.iter().map(|r| r.team).collect();
        team_dataset.write_raw(&team_data)?;

        // practice: [n_regions] u8 (0 or 1)
        let practice_dataset = regions_group
            .new_dataset::<u8>()
            .shape([n_regions])
            .create("practice")?;
        let practice_data: Vec<u8> = seq
            .active_regions
            .iter()
            .map(|r| if r.practice { 1 } else { 0 })
            .collect();
        practice_dataset.write_raw(&practice_data)?;

        // end_reason: [n_regions] u8 (enum as int)
        let end_reason_dataset = regions_group
            .new_dataset::<u8>()
            .shape([n_regions])
            .create("end_reason")?;
        let end_reason_data: Vec<u8> = seq
            .active_regions
            .iter()
            .map(|r| r.end_reason.as_u8())
            .collect();
        end_reason_dataset.write_raw(&end_reason_data)?;

        // Add attribute with enum names for reference
        let end_reason_names_attr = regions_group
            .new_attr::<hdf5::types::VarLenAscii>()
            .create("end_reason_names")?;
        end_reason_names_attr
            .write_scalar(&hdf5::types::VarLenAscii::from_ascii(RegionEndReason::names())?)?;

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
