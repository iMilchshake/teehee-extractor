use crate::features::FeatureSet;
use crate::sequence::{PlayerSequence, RegionEndReason};
use anyhow::{Context, Result};
use arrow::array::*;
use arrow::datatypes::{DataType, Field, Schema};
use arrow::ipc::writer::{FileWriter, IpcWriteOptions};
use arrow::ipc::{CompressionType, MetadataVersion};
use arrow::record_batch::RecordBatch;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;
use std::sync::Arc;

/// Streaming Arrow IPC writer. Each sequence becomes a RecordBatch written
/// immediately to disk — no metadata accumulation, constant memory.
pub struct ArrowIpcWriter {
    writer: FileWriter<BufWriter<File>>,
    schema: Arc<Schema>,
    feature_set: FeatureSet,
    seq_idx: u32,
}

impl ArrowIpcWriter {
    pub fn create(output_path: &Path, feature_set: FeatureSet) -> Result<Self> {
        let schema = Arc::new(build_schema(&feature_set));

        let file = File::create(output_path)
            .with_context(|| format!("failed to create file: {}", output_path.display()))?;
        let options = IpcWriteOptions::try_new(8, false, MetadataVersion::V5)?
            .try_with_compression(Some(CompressionType::LZ4_FRAME))?;
        let writer = FileWriter::try_new_with_options(BufWriter::new(file), &schema, options)?;

        Ok(Self {
            writer,
            schema,
            feature_set,
            seq_idx: 0,
        })
    }

    /// Write a single PlayerSequence as a RecordBatch.
    pub fn write_sequence(&mut self, seq: &PlayerSequence) -> Result<usize> {
        if seq.data.is_empty() {
            return Ok(self.seq_idx as usize);
        }

        let idx = self.seq_idx;
        let n_ticks = seq.data.len();

        // metadata columns (constant per sequence, repeated per tick)
        let seq_id_arr = UInt32Array::from(vec![idx; n_ticks]);
        let player_name_arr = StringArray::from(vec![seq.player_name.as_str(); n_ticks]);
        let team_arr = Int32Array::from(vec![seq.team; n_ticks]);
        let map_name_arr = StringArray::from(vec![seq.map_name.as_str(); n_ticks]);
        let time_of_day_arr = StringArray::from(vec![seq.time_of_day.as_str(); n_ticks]);
        let timeout_code_arr =
            StringArray::from(vec![seq.timeout_code.as_deref().unwrap_or(""); n_ticks]);

        // tick column
        let tick_arr = Int64Array::from(seq.data.iter().map(|t| t.tick).collect::<Vec<_>>());

        // map each tick to its region
        let mut region_ids = vec![0u32; n_ticks];
        let mut region_teams = vec![0i32; n_ticks];
        let mut region_practices = vec![false; n_ticks];
        let mut region_end_reasons = vec![RegionEndReason::Unknown; n_ticks];

        for (reg_idx, region) in seq.active_regions.iter().enumerate() {
            if let (Some(start), Some(end)) = (region.start_idx, region.end_idx) {
                for i in start..end.min(n_ticks) {
                    region_ids[i] = reg_idx as u32;
                    region_teams[i] = region.team;
                    region_practices[i] = region.practice;
                    region_end_reasons[i] = region.end_reason;
                }
            }
        }

        let region_id_arr = UInt32Array::from(region_ids);
        let region_team_arr = Int32Array::from(region_teams);
        let region_practice_arr = BooleanArray::from(region_practices);
        let region_end_reason_arr = UInt8Array::from(
            region_end_reasons
                .iter()
                .map(|r| r.as_u8())
                .collect::<Vec<_>>(),
        );

        // feature columns
        let feature_arrays: Vec<Arc<dyn Array>> = self
            .feature_set
            .features()
            .iter()
            .map(|feature| {
                let values: Vec<f32> = seq
                    .data
                    .iter()
                    .map(|tick| self.feature_set.extract_single(tick, *feature))
                    .collect();
                Arc::new(Float32Array::from(values)) as Arc<dyn Array>
            })
            .collect();

        // build columns in schema order
        let mut columns: Vec<Arc<dyn Array>> = vec![
            Arc::new(seq_id_arr),
            Arc::new(player_name_arr),
            Arc::new(team_arr),
            Arc::new(map_name_arr),
            Arc::new(time_of_day_arr),
            Arc::new(timeout_code_arr),
            Arc::new(tick_arr),
            Arc::new(region_id_arr),
            Arc::new(region_team_arr),
            Arc::new(region_practice_arr),
            Arc::new(region_end_reason_arr),
        ];
        columns.extend(feature_arrays);

        let batch = RecordBatch::try_new(self.schema.clone(), columns)?;
        self.writer.write(&batch)?;

        self.seq_idx += 1;
        Ok(idx as usize)
    }

    /// Finish writing (writes footer).
    pub fn finish(mut self) -> Result<usize> {
        self.writer.finish()?;
        Ok(self.seq_idx as usize)
    }

    pub fn sequences_written(&self) -> usize {
        self.seq_idx as usize
    }
}

fn build_schema(feature_set: &FeatureSet) -> Schema {
    let mut fields = vec![
        Field::new("seq_id", DataType::UInt32, false),
        Field::new("player_name", DataType::Utf8, false),
        Field::new("team", DataType::Int32, false),
        Field::new("map_name", DataType::Utf8, false),
        Field::new("time_of_day", DataType::Utf8, false),
        Field::new("timeout_code", DataType::Utf8, false),
        Field::new("tick", DataType::Int64, false),
        Field::new("region_id", DataType::UInt32, false),
        Field::new("region_team", DataType::Int32, false),
        Field::new("region_practice", DataType::Boolean, false),
        Field::new("region_end_reason", DataType::UInt8, false),
    ];

    for feature in feature_set.features() {
        fields.push(Field::new(feature.name(), DataType::Float32, false));
    }

    let mut metadata = std::collections::HashMap::new();
    metadata.insert(
        "end_reason_names".to_string(),
        RegionEndReason::names().to_string(),
    );

    Schema::new_with_metadata(fields, metadata)
}
