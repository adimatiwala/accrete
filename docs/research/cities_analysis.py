"""Print paired treatment results from the Accrete cities factorial sweep."""

import argparse
import csv
from itertools import accumulate
from pathlib import Path
from statistics import mean


ARMS = {
    (False, False): "Control",
    (True, False): "Markets",
    (False, True): "Shared roads",
    (True, True): "Both",
}

OUTCOMES = {
    "trade_rate": ("Trades per 1,000 person-days", 1),
    "trades_per_day": ("Trades per world-day", 1),
    "same_tile": ("Recent trade pairs on the same tile (%)", 2),
    "road_tiles": ("Road tiles", 1),
    "specialization": ("Specialisation", 3),
    "largest_settlement": ("Residents in largest tracked group", 1),
    "cluster_share": ("Homed residents in largest cluster (%)", 1),
    "population": ("Population", 1),
    "birth_rate": ("Births per 1,000 adult-days", 1),
    "market_hours": ("Agent-hours on market goals (%)", 1),
    "rest_hours": ("Agent-hours on rest goals (%)", 1),
    "food_hours": ("Agent-hours on food-gathering goals (%)", 1),
    "village_days": ("Days with a village or town (full run)", 0),
    "hit_cap": ("Runs reaching the population cap (%)", 0),
}


def read_csv(path):
    with path.open(encoding="utf-8", newline="") as file:
        return list(csv.DictReader(file))


def measure(rows, end_day, window):
    rows = [row for row in rows if int(row["day"]) <= end_day]
    if [int(row["day"]) for row in rows] != list(range(1, end_day + 1)):
        raise ValueError(f"expected a metric row for each of days 1–{end_day}")
    tail = rows[-window:]
    person_days = sum(int(row["population"]) for row in tail)
    adult_days = sum(int(row["adults"]) for row in tail)
    trade_rate = 1000 * sum(int(row["trades"]) for row in tail) / max(1, person_days)
    trades = list(accumulate((int(row["trades"]) for row in rows), initial=0))
    same_tile = []
    for day, row in enumerate(tail, start=end_day - window + 1):
        count = trades[day] - trades[max(0, day - 30)]
        if count >= 2:
            hhi = float(row["trade_tile_hhi_30d"])
            same_tile.append(max(0, (count * hhi - 1) / (count - 1)))
    return {
        "trade_rate": trade_rate,
        "trades_per_day": mean(int(row["trades"]) for row in tail),
        "same_tile": 100 * mean(same_tile) if same_tile else 0.0,
        "road_tiles": mean(int(row["road_tiles"]) for row in tail),
        "specialization": mean(float(row["specialization"]) for row in tail),
        "largest_settlement": mean(int(row["largest_settlement"]) for row in tail),
        "cluster_share": 100 * mean(float(row["largest_cluster_share"]) for row in tail),
        "population": mean(int(row["population"]) for row in tail),
        "birth_rate": 1000 * sum(int(row["births"]) for row in tail) / max(1, adult_days),
        "market_hours": 100 * mean(float(row["goal_market"]) for row in tail),
        "rest_hours": 100 * mean(float(row["goal_rest"]) for row in tail),
        "food_hours": 100 * mean(float(row["goal_gather_food"]) for row in tail),
        "village_days": sum(int(row["villages"]) + int(row["towns"]) > 0 for row in rows),
        "hit_cap": 100 * any(row["at_cap"] == "true" for row in rows),
    }


def load(directory, end_day, window):
    runs = {}
    for row in read_csv(directory / "sweep_runs.csv"):
        markets = row["markets.enabled"]
        roads = float(row["infrastructure.public_weight"])
        if markets not in ("false", "true") or roads not in (0.0, 1.0):
            raise ValueError("expected the 2×2 markets and shared roads design")
        arm = markets == "true", roads == 1.0
        seed, point = int(row["seed"]), int(row["point"])
        block = runs.setdefault(seed, {})
        if arm in block:
            raise ValueError(f"duplicate {ARMS[arm]} run for seed {seed}")
        metrics_path = directory / f"p{point:03}_seed{seed}_metrics.csv"
        block[arm] = measure(read_csv(metrics_path), end_day, window)
    if not runs or any(set(block) != set(ARMS) for block in runs.values()):
        raise ValueError("every seed must have all four arms")
    return runs


def report(runs):
    print("| Outcome | " + " | ".join(ARMS.values()) + " |")
    print("| --- | ---: | ---: | ---: | ---: |")
    for key, (label, places) in OUTCOMES.items():
        means = [mean(block[arm][key] for block in runs.values()) for arm in ARMS]
        print(f"| {label} | " + " | ".join(f"{v:.{places}f}" for v in means) + " |")

    print()
    print(
        "| Outcome | Markets minus control | Roads minus control "
        "| Both minus control | Interaction |"
    )
    print("| --- | ---: | ---: | ---: | ---: |")
    for key, (label, places) in OUTCOMES.items():
        contrasts = [[], [], [], []]
        for block in runs.values():
            control, markets, roads, both = (block[arm][key] for arm in ARMS)
            differences = (
                markets - control,
                roads - control,
                both - control,
                both - markets - roads + control,
            )
            for values, contrast in zip(contrasts, differences):
                values.append(contrast)
        cells = [
            f"{mean(values):+.{places}f} [{min(values):+.{places}f}, {max(values):+.{places}f}]"
            for values in contrasts
        ]
        print(f"| {label} | " + " | ".join(cells) + " |")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--end-day", type=int, default=1000)
    parser.add_argument("--window", type=int, default=100)
    args = parser.parse_args()
    if not 1 <= args.window <= args.end_day:
        parser.error("window must be between 1 and the end day")
    report(load(args.directory, args.end_day, args.window))


if __name__ == "__main__":
    main()
