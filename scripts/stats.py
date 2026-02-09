#!/usr/bin/env python3
"""Statistical analysis of extracted parquet dataset."""

import argparse
import sys
from pathlib import Path

import polars as pl


END_REASONS = {
    0: "Kill",
    1: "TeamChange",
    2: "SwapTees",
    3: "Spectate",
    4: "Afk",
    5: "Leave",
    6: "ChangeMap",
    7: "Unknown",
}


def load_dataset(path: Path) -> pl.LazyFrame:
    if path.is_dir():
        return pl.scan_parquet(path / "*.parquet")
    return pl.scan_parquet(path)


def fmt_pct(n: int, total: int) -> str:
    return f"{n:>12,} ticks ({n / total * 100:5.1f}%)" if total > 0 else "0"


def fmt_duration(ticks: int) -> str:
    secs = ticks / 50
    if secs < 60:
        return f"{secs:.0f}s"
    if secs < 3600:
        return f"{secs / 60:.1f}m"
    return f"{secs / 3600:.1f}h"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("path", type=Path, help="parquet file or directory of parquet files")
    args = parser.parse_args()

    if not args.path.exists():
        print(f"Error: {args.path} does not exist", file=sys.stderr)
        sys.exit(1)

    lf = load_dataset(args.path)
    schema = lf.collect_schema()
    n_files = len(list(args.path.glob("*.parquet"))) if args.path.is_dir() else 1

    # --- single pass: overview + practice + end reasons + teams ---
    overview = (
        lf.group_by("region_end_reason", "region_practice", "region_team")
        .agg(pl.len().alias("ticks"))
        .collect(streaming=True)
    )
    total_ticks = overview["ticks"].sum()

    # practice mode
    practice_ticks = overview.filter(pl.col("region_practice") == True)["ticks"].sum()
    normal_ticks = total_ticks - practice_ticks

    # end reasons (ticks)
    reason_ticks = (
        overview.group_by("region_end_reason")
        .agg(pl.col("ticks").sum())
        .sort("ticks", descending=True)
    )

    # teams
    team_counts = (
        overview.group_by("region_team")
        .agg(pl.col("ticks").sum().alias("ticks"))
        .sort("ticks", descending=True)
    )

    del overview

    # --- region counts: deduplicate to one row per region, then count ---
    region_summary = (
        lf.select("seq_id", "region_id", "region_end_reason", "map_name")
        .unique()
        .group_by("region_end_reason")
        .agg(pl.len().alias("regions"))
        .collect(streaming=True)
    )

    # merge region counts with tick counts
    reason_merged = reason_ticks.join(region_summary, on="region_end_reason", how="left")
    total_regions = reason_merged["regions"].sum()

    del region_summary, reason_ticks

    # --- sequence / player / map counts (small columns only) ---
    seq_info = (
        lf.select("seq_id", "player_name", "map_name")
        .unique()
        .collect(streaming=True)
    )
    n_sequences = len(seq_info)
    n_players = seq_info["player_name"].n_unique()
    n_maps = seq_info["map_name"].n_unique()

    del seq_info

    # --- print overview ---
    print("=" * 60)
    print("DATASET OVERVIEW")
    print("=" * 60)
    print(f"  Files:          {n_files:>12,}")
    print(f"  Sequences:      {n_sequences:>12,}")
    print(f"  Total ticks:    {total_ticks:>12,}  ({fmt_duration(total_ticks)})")
    print(f"  Unique players: {n_players:>12,}")
    print(f"  Unique maps:    {n_maps:>12,}")

    print()
    print("=" * 60)
    print("PRACTICE MODE")
    print("=" * 60)
    print(f"  Normal:   {fmt_pct(normal_ticks, total_ticks)}")
    print(f"  Practice: {fmt_pct(practice_ticks, total_ticks)}")

    print()
    print("=" * 60)
    print("REGION END REASONS")
    print("=" * 60)
    print(f"  {'reason':<12} {'regions':>10}  {'% regions':>8}  {'ticks':>14}  {'% ticks':>7}")
    print(f"  {'-'*12} {'-'*10}  {'-'*8}  {'-'*14}  {'-'*7}")
    for row in reason_merged.sort("ticks", descending=True).iter_rows():
        reason_id, ticks, regions = row
        name = END_REASONS.get(reason_id, f"?{reason_id}")
        rpct = regions / total_regions * 100 if total_regions else 0
        tpct = ticks / total_ticks * 100 if total_ticks else 0
        print(f"  {name:<12} {regions:>10,}  {rpct:>7.1f}%  {ticks:>14,}  {tpct:>6.1f}%")

    del reason_merged

    print()
    print("=" * 60)
    print("TEAM DISTRIBUTION (ticks per team)")
    print("=" * 60)
    for row in team_counts.head(15).iter_rows():
        team, ticks = row
        print(f"  Team {team:<5} {fmt_pct(ticks, total_ticks)}")
    if len(team_counts) > 15:
        print(f"  ... and {len(team_counts) - 15} more teams")

    del team_counts

    # --- top maps ---
    map_counts = (
        lf.group_by("map_name")
        .agg(pl.len().alias("ticks"))
        .sort("ticks", descending=True)
        .head(21)
        .collect(streaming=True)
    )
    n_other_maps = n_maps - min(20, len(map_counts))

    print()
    print("=" * 60)
    print("TOP 20 MAPS (by tick count)")
    print("=" * 60)
    for row in map_counts.head(20).iter_rows():
        name, ticks = row
        print(f"  {name:<30} {fmt_pct(ticks, total_ticks)}")
    if n_other_maps > 0:
        print(f"  ... and {n_other_maps} more maps")

    del map_counts

    # --- top players ---
    player_counts = (
        lf.group_by("player_name")
        .agg(pl.len().alias("ticks"))
        .sort("ticks", descending=True)
        .head(21)
        .collect(streaming=True)
    )
    n_other_players = n_players - min(20, len(player_counts))

    print()
    print("=" * 60)
    print("TOP 20 PLAYERS (by tick count)")
    print("=" * 60)
    for row in player_counts.head(20).iter_rows():
        name, ticks = row
        print(f"  {name:<30} {fmt_pct(ticks, total_ticks)}")
    if n_other_players > 0:
        print(f"  ... and {n_other_players} more players")

    del player_counts

    # --- feature ranges (streaming min/mean/max in one pass) ---
    feature_cols = [
        c for c in schema.names()
        if c not in {
            "seq_id", "player_name", "team", "map_name", "time_of_day",
            "timeout_code", "tick", "region_id", "region_team",
            "region_practice", "region_end_reason",
        }
    ]

    if feature_cols:
        stats = (
            lf.select(
                *[pl.col(c).min().alias(f"{c}_min") for c in feature_cols],
                *[pl.col(c).mean().alias(f"{c}_mean") for c in feature_cols],
                *[pl.col(c).max().alias(f"{c}_max") for c in feature_cols],
            )
            .collect(streaming=True)
        )

        print()
        print("=" * 60)
        print("FEATURE RANGES")
        print("=" * 60)
        print(f"  {'feature':<20} {'min':>12} {'mean':>12} {'max':>12}")
        print(f"  {'-'*20} {'-'*12} {'-'*12} {'-'*12}")
        for c in feature_cols:
            mn = stats[f"{c}_min"][0]
            avg = stats[f"{c}_mean"][0]
            mx = stats[f"{c}_max"][0]
            print(f"  {c:<20} {mn:>12.2f} {avg:>12.2f} {mx:>12.2f}")

        del stats

    # --- disk size ---
    if args.path.is_dir():
        total_bytes = sum(f.stat().st_size for f in args.path.glob("*.parquet"))
    else:
        total_bytes = args.path.stat().st_size

    bytes_per_tick = total_bytes / total_ticks if total_ticks > 0 else 0

    print()
    print("=" * 60)
    print("STORAGE")
    print("=" * 60)
    print(f"  Total size:     {total_bytes / 1e9:>10.2f} GB")
    print(f"  Bytes per tick: {bytes_per_tick:>10.1f}")
    print()


if __name__ == "__main__":
    main()
