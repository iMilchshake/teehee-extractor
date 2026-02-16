# TODO

## Active Region Tracking
- [x] add active_regions split on player kill
- [x] add practice detection
- [x] block /practice for team 0
- [x] filter out empty regions after AFK removal
- [x] practice inheritance for new team members (team-level tracking)

## Resolved
- **Kill region breaks**: `/kill` is sent as `ClNetMessage::ClKill` (net message), not
  `Command::Kill` (console command). Handled in `on_net_msg`.
- **Spectate gaps**: Player disappears from `snap_buf.players` during spectate, creating
  a natural tick gap detected by `split_regions_on_gaps`.

## Future
- detect crossing start line?
- improve player identity with timeout codes 
- 
