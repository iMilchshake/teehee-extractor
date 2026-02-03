#!/usr/bin/env python3
"""Interactive visualization of extracted teehistorian data from HDF5 files."""

import argparse
import sys

import h5py
import numpy as np
import matplotlib.pyplot as plt
from matplotlib.animation import FuncAnimation
from matplotlib.widgets import Slider, Button


def list_sequences(f: h5py.File) -> None:
    """Print summary of all sequences in the file."""
    print(f"{'Seq':<8} {'Player':<20} {'Ticks':<10} {'Finishes':<10} {'Map'}")
    print("-" * 70)
    for name in sorted(f.keys(), key=lambda x: int(x.split("_")[1])):
        grp = f[name]
        player = grp.attrs["player_name"]
        ticks = grp["data"].shape[0]
        finishes = grp["finishes"].shape[0]
        map_name = grp.attrs["map_name"]
        print(f"{name:<8} {player:<20} {ticks:<10} {finishes:<10} {map_name}")


def get_feature_idx(features: list[str], name: str) -> int:
    """Get index of a feature by name."""
    return features.index(name)


def plot_sequence(filepath: str, seq_name: str, speed: int = 1) -> None:
    """Create interactive visualization with animation."""
    with h5py.File(filepath, "r") as f:
        features = f.attrs["feature_names"].split(",")
        grp = f[seq_name]
        data = grp["data"][:]
        player = grp.attrs["player_name"]
        map_name = grp.attrs["map_name"]
        finishes = grp["finishes"][:]

    n_ticks = len(data)
    finish_str = f"{len(finishes)} finish(es)" if len(finishes) > 0 else "no finish"

    # Extract features
    pos_x = data[:, get_feature_idx(features, "pos_x")]
    pos_y = data[:, get_feature_idx(features, "pos_y")]
    move_dir = data[:, get_feature_idx(features, "move_dir")]
    cursor_x = data[:, get_feature_idx(features, "cursor_x")]
    cursor_y = data[:, get_feature_idx(features, "cursor_y")]
    key_hook = data[:, get_feature_idx(features, "key_hook")]
    hook_grabbed = data[:, get_feature_idx(features, "hook_grabbed")]
    hook_pos_x = data[:, get_feature_idx(features, "hook_pos_x")]
    hook_pos_y = data[:, get_feature_idx(features, "hook_pos_y")]
    freeze_status = data[:, get_feature_idx(features, "freeze_status")]

    # Create figure
    fig, ax = plt.subplots(figsize=(12, 10))
    plt.subplots_adjust(bottom=0.2)

    ax.set_title(f"{player} on {map_name} ({n_ticks} ticks, {finish_str})")
    ax.set_xlabel("pos_x")
    ax.set_ylabel("pos_y")
    ax.set_aspect("equal")
    ax.invert_yaxis()

    # Static background - full trajectory
    ax.plot(pos_x, pos_y, color="lightgray", linewidth=0.5, zorder=1)

    # Static markers
    left_mask = move_dir < 0
    right_mask = move_dir > 0
    neutral_mask = move_dir == 0

    ax.scatter(pos_x[left_mask], pos_y[left_mask], marker="<", s=8, c="blue", alpha=0.3, zorder=2)
    ax.scatter(pos_x[right_mask], pos_y[right_mask], marker=">", s=8, c="blue", alpha=0.3, zorder=2)
    ax.scatter(pos_x[neutral_mask], pos_y[neutral_mask], marker="o", s=4, c="gray", alpha=0.2, zorder=2)

    # Static cursor lines (faded)
    hook_mask = key_hook > 0
    for i in np.where(hook_mask)[0]:
        ax.plot([pos_x[i], pos_x[i] + cursor_x[i]], [pos_y[i], pos_y[i] + cursor_y[i]],
                color="red", linewidth=0.3, alpha=0.1, zorder=2)

    # Static hook lines (faded)
    for i in np.where(hook_grabbed > 0)[0]:
        ax.plot([pos_x[i], hook_pos_x[i]], [pos_y[i], hook_pos_y[i]],
                color="green", linewidth=0.5, alpha=0.2, zorder=2)

    # Animated elements
    current_pos, = ax.plot([], [], "o", markersize=12, color="gold", markeredgecolor="black",
                           markeredgewidth=2, zorder=10)
    current_hook, = ax.plot([], [], "-", linewidth=3, color="lime", zorder=9)
    current_cursor, = ax.plot([], [], "-", linewidth=2, color="orange", zorder=9)
    cursor_marker, = ax.plot([], [], "x", markersize=8, color="orange", zorder=9)
    tick_text = ax.text(0.02, 0.98, "", transform=ax.transAxes, fontsize=10,
                        verticalalignment="top", fontfamily="monospace",
                        bbox=dict(boxstyle="round", facecolor="white", alpha=0.8))

    # Animation state
    state = {"frame": 0, "playing": False, "speed": speed}

    def update_frame(frame_idx):
        """Update animated elements for given frame."""
        t = frame_idx % n_ticks

        # Current position (black if frozen)
        current_pos.set_data([pos_x[t]], [pos_y[t]])
        current_pos.set_color("black" if freeze_status[t] > 0 else "gold")

        # Current hook
        if hook_grabbed[t] > 0:
            current_hook.set_data([pos_x[t], hook_pos_x[t]], [pos_y[t], hook_pos_y[t]])
        else:
            current_hook.set_data([], [])

        # Current cursor
        if key_hook[t] > 0:
            cx = pos_x[t] + cursor_x[t]
            cy = pos_y[t] + cursor_y[t]
            current_cursor.set_data([pos_x[t], cx], [pos_y[t], cy])
            cursor_marker.set_data([cx], [cy])
        else:
            current_cursor.set_data([], [])
            cursor_marker.set_data([], [])

        # Tick display
        tick_text.set_text(f"Tick: {t}/{n_ticks-1}\nMove: {int(move_dir[t]):+d}")

        return current_pos, current_hook, current_cursor, cursor_marker, tick_text

    def animate(frame):
        """Animation function called each frame."""
        if state["playing"]:
            state["frame"] = (state["frame"] + state["speed"]) % n_ticks
            # Update slider without triggering callback (causes lag)
            slider.val = state["frame"]
            slider.valtext.set_text(f"{state['frame']}")
        return update_frame(state["frame"])

    # Slider
    ax_slider = plt.axes([0.2, 0.08, 0.6, 0.03])
    slider = Slider(ax_slider, "Tick", 0, n_ticks - 1, valinit=0, valstep=1)

    def on_slider(val):
        state["frame"] = int(val)
        update_frame(state["frame"])
        fig.canvas.draw_idle()

    slider.on_changed(on_slider)

    # Play/Pause button
    ax_play = plt.axes([0.2, 0.02, 0.1, 0.04])
    btn_play = Button(ax_play, "▶ Play")

    def on_play(event):
        state["playing"] = not state["playing"]
        btn_play.label.set_text("⏸ Pause" if state["playing"] else "▶ Play")

    btn_play.on_clicked(on_play)

    # Speed buttons
    ax_slower = plt.axes([0.35, 0.02, 0.08, 0.04])
    btn_slower = Button(ax_slower, "Slower")

    ax_faster = plt.axes([0.45, 0.02, 0.08, 0.04])
    btn_faster = Button(ax_faster, "Faster")

    ax_speed = plt.axes([0.55, 0.02, 0.12, 0.04])
    speed_text = plt.text(0.5, 0.5, f"{state['speed']}x (50 t/s)", transform=ax_speed.transAxes,
                          ha="center", va="center", fontsize=9)
    ax_speed.set_xticks([])
    ax_speed.set_yticks([])

    def on_slower(event):
        state["speed"] = max(1, state["speed"] // 2)
        speed_text.set_text(f"{state['speed']}x ({state['speed'] * 50} t/s)")

    def on_faster(event):
        state["speed"] = min(50, state["speed"] * 2)
        speed_text.set_text(f"{state['speed']}x ({state['speed'] * 50} t/s)")

    btn_slower.on_clicked(on_slower)
    btn_faster.on_clicked(on_faster)

    # Reset button
    ax_reset = plt.axes([0.7, 0.02, 0.1, 0.04])
    btn_reset = Button(ax_reset, "Reset")

    def on_reset(event):
        state["frame"] = 0
        state["playing"] = False
        btn_play.label.set_text("▶ Play")
        slider.set_val(0)

    btn_reset.on_clicked(on_reset)

    # Scroll-wheel zoom
    def on_scroll(event):
        if event.inaxes != ax:
            return
        scale = 0.8 if event.button == "up" else 1.25
        xlim = ax.get_xlim()
        ylim = ax.get_ylim()
        xdata, ydata = event.xdata, event.ydata
        new_xlim = [xdata - (xdata - xlim[0]) * scale, xdata + (xlim[1] - xdata) * scale]
        new_ylim = [ydata - (ydata - ylim[0]) * scale, ydata + (ylim[1] - ydata) * scale]
        ax.set_xlim(new_xlim)
        ax.set_ylim(new_ylim)
        fig.canvas.draw_idle()

    fig.canvas.mpl_connect("scroll_event", on_scroll)

    # Animation
    anim = FuncAnimation(fig, animate, interval=20, blit=True, cache_frame_data=False)

    # Initial frame
    update_frame(0)

    plt.show()


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Interactive visualization of extracted teehistorian data."
    )
    parser.add_argument("input", help="Input HDF5 file")
    parser.add_argument(
        "-s", "--seq",
        help="Sequence to visualize (e.g., 'seq_0' or just '0'). If not provided, lists all sequences.",
    )
    parser.add_argument(
        "--speed",
        type=int,
        default=1,
        help="Initial playback speed multiplier (default: 1)",
    )

    args = parser.parse_args()

    # List mode
    if not args.seq:
        with h5py.File(args.input, "r") as f:
            list_sequences(f)
        return

    # Visualize mode
    seq_name = args.seq if args.seq.startswith("seq_") else f"seq_{args.seq}"

    with h5py.File(args.input, "r") as f:
        if seq_name not in f:
            print(f"Error: Sequence '{seq_name}' not found.", file=sys.stderr)
            print(f"Available: {list(f.keys())}", file=sys.stderr)
            sys.exit(1)

    plot_sequence(args.input, seq_name, args.speed)


if __name__ == "__main__":
    main()
