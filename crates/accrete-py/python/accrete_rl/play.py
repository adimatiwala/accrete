"""Watch all residents act using the same learned Accrete policy."""

import argparse
from collections import defaultdict
from pathlib import Path

import numpy as np

from . import _core
from .env import ACTIONS


class Playback:
    def __init__(self, model, seed, weather, check):
        self.model = model
        self.seed = seed
        self.weather = weather
        self.check = check
        self.reset()

    def reset(self):
        import torch

        torch.manual_seed(self.seed)
        self.world = _core.NativeLearnedWorld(self.seed, self.weather, self.check)
        self.choices = {}

    def tick(self):
        frames = self.world.observations()
        if not frames:
            return False
        ids, observations = zip(*frames)
        states, targets, masks = zip(*observations)
        mask = np.asarray(masks, dtype=np.bool_)
        obs = {
            "state": np.asarray(states, dtype=np.float32),
            "targets": np.asarray(targets, dtype=np.float32),
            "action_mask": mask.astype(np.int8),
        }
        actions, _ = self.model.predict(obs, action_masks=mask, deterministic=False)
        choices = actions.tolist()
        self.world.step(choices)
        self.choices = dict(zip(ids, (ACTIONS[action].replace("_", " ") for action in choices)))
        return True


def show(playback):
    import tkinter as tk
    from tkinter import ttk

    root = tk.Tk()
    root.title("Accrete | Learned Residents")
    root.configure(background="#101923")
    root.resizable(False, False)
    shell = ttk.Frame(root, padding=16)
    shell.grid()
    status = ttk.Label(shell, font=("Segoe UI", 13))
    status.grid(row=0, column=0, columnspan=2, sticky="w", pady=(0, 12))
    cell = 45
    canvas = tk.Canvas(shell, width=17 * cell, height=9 * cell, highlightthickness=0)
    canvas.grid(row=1, column=0)
    residents = tk.Canvas(shell, width=237, height=9 * cell, bg="#172432", highlightthickness=0)
    residents.grid(row=1, column=1, padx=(12, 0))
    controls = ttk.Frame(shell)
    controls.grid(row=2, column=0, sticky="w", pady=(12, 0))
    running = True
    speed = tk.IntVar(value=4)
    colors = ("#60daf4", "#ffd17a", "#e5a1f6", "#8de4a4", "#fa958d")
    terrain = ("#304b60", "#334f43", "#254537", "#535a45", "#696d6e")

    def paint():
        tick, width, _, tiles, homes, agents, trades = playback.world.display()
        status.configure(
            text=f"Day {tick // 24 + 1}  ·  {tick % 24:02}:00  ·  "
                 f"{len(agents)} learned residents  ·  {trades} trades  ·  Seed {playback.seed}"
        )
        canvas.delete("all")
        residents.delete("all")
        residents.create_text(16, 19, anchor="w", text="RESIDENTS", fill="#e8f0f5", font=("Segoe UI", 10, "bold"))
        for index, (biome, road, food, wood) in enumerate(tiles):
            left, top = (index % width) * cell, (index // width) * cell
            canvas.create_rectangle(left, top, left + cell, top + cell, fill=terrain[biome], outline="#2b4038")
            x, y = left + cell / 2, top + cell / 2
            if road:
                color = "#aba99d" if road == 2 else "#80775c"
                canvas.create_line(left, y, left + cell, y, width=6, fill=color)
            if food > 0.05:
                radius = min(12, 2 + food / 2)
                canvas.create_oval(x - radius, y - radius, x + radius, y + radius, fill="#a9d060", outline="")
            if wood > 0.05:
                canvas.create_polygon(x, y - 12, x - 9, y + 4, x + 9, y + 4, fill="#84a985", outline="")
        for home, col, row, stock in homes:
            x, y = col * cell + cell / 2, row * cell + cell / 2
            canvas.create_rectangle(x - 14, y - 6, x + 14, y + 12, fill="#e0ba83", outline="#30271d")
            canvas.create_polygon(x - 17, y - 5, x, y - 16, x + 17, y - 5, fill="#d68663")
            label = "Market" if home == 1 else f"Chest {stock:.1f}"
            canvas.create_text(x, y + 30, text=label, fill="#fff1d6", font=("Segoe UI", 10, "bold"))
        occupants = defaultdict(int)
        for index, (agent, col, row, hunger, energy, health, food) in enumerate(agents):
            offset = occupants[col, row]
            occupants[col, row] += 1
            x, y = col * cell + 12 + 10 * (offset % 3), row * cell + 12 + 13 * (offset // 3)
            color = colors[agent % len(colors)]
            canvas.create_oval(x - 9, y - 9, x + 9, y + 9, fill=color, outline="#141d24")
            canvas.create_text(x, y, text=str(agent + 1), fill="#16232a", font=("Segoe UI", 8, "bold"))
            top = 45 + index * 65
            action = playback.choices.get(agent, "ready")
            residents.create_text(16, top, anchor="w", text=f"{agent + 1}   {action.title()}", fill=color,
                                  font=("Segoe UI", 11, "bold"))
            residents.create_text(
                16, top + 21, anchor="w",
                text=f"Food {food:.1f}   Health {health:.0%}   Energy {energy:.0%}",
                fill="#ccd8e0", font=("Segoe UI", 9),
            )
            residents.create_rectangle(16, top + 36, 218, top + 41, fill="#2f4354", outline="")
            residents.create_rectangle(16, top + 36, 16 + 202 * hunger, top + 41, fill="#d7835d", outline="")
        if not agents:
            residents.create_text(16, 65, anchor="w", text="All residents died.", fill="#e8f0f5")

    def toggle():
        nonlocal running
        running = not running
        pause.configure(text="Pause" if running else "Play")

    def step_once():
        if running:
            toggle()
        playback.tick()
        paint()

    def restart():
        playback.reset()
        paint()

    def loop():
        if running:
            if not playback.tick():
                toggle()
            paint()
        root.after(1000 // max(1, speed.get()), loop)

    pause = ttk.Button(controls, text="Pause", command=toggle)
    pause.pack(side="left")
    ttk.Button(controls, text="Step", command=step_once).pack(side="left", padx=6)
    ttk.Button(controls, text="Restart", command=restart).pack(side="left")
    ttk.Label(controls, text="Hours per second").pack(side="left", padx=(16, 6))
    ttk.Scale(controls, from_=1, to=20, variable=speed).pack(side="left")
    ttk.Label(shell, text="Green circles  Food  ·  Orange bars  Hunger").grid(
        row=2, column=1, sticky="w", pady=(12, 0)
    )
    root.bind("<space>", lambda event: toggle())
    paint()
    root.after(1000, loop)
    root.mainloop()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--model", type=Path, required=True)
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--weather", choices=("mixed", "steady", "drought"), default="drought")
    parser.add_argument("--smoke", type=int, metavar="TICKS", help="run a checked headless preview")
    args = parser.parse_args()
    import torch
    from sb3_contrib import MaskablePPO

    torch.set_num_threads(1)
    playback = Playback(MaskablePPO.load(args.model, device="cpu"), args.seed, args.weather, args.smoke is not None)
    if args.smoke is not None:
        for _ in range(args.smoke):
            if not playback.tick():
                break
        tick, _, _, _, _, residents, trades = playback.world.display()
        print(f"{tick} ticks, {len(residents)} living, {trades} trades")
    else:
        show(playback)


if __name__ == "__main__":
    main()
