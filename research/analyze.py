#!/usr/bin/env python3
"""Analyse the research sweep CSV and render the charts used in docs/research.md.

Reads research/data/results.csv (produced by `cargo run -- research`) and writes
three PNGs to research/charts/. Pure analysis — no network, deterministic.

Runs flagged as unrealistic by `pool_share` (the modelled position would be a
large fraction of pool liquidity, where the constant-L fee approximation
overstates fees) are excluded from quantitative charts; see the SHARE_CAP note.

Usage:  python research/analyze.py
"""
import os
import pandas as pd
import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt

HERE = os.path.dirname(os.path.abspath(__file__))
CSV = os.path.join(HERE, "data", "results.csv")
CHARTS = os.path.join(HERE, "charts")
# Above this position-to-pool liquidity share the constant-L approximation
# overstates fees, so those runs are excluded from quantitative claims.
SHARE_CAP = 0.25
HEADLINE_WIDTH = 0.10  # ±10% for the cross-pool comparison
FOCUS_POOL = "SOL/USDC"  # deep pool for the range-width sweep

os.makedirs(CHARTS, exist_ok=True)
df = pd.read_csv(CSV)
df["fees_pct"] = df["total_fees_usd"] / df["capital_usd"] * 100
df["il_pct"] = df["il_usd"] / df["capital_usd"] * 100
realistic = df[df["pool_share"] < SHARE_CAP].copy()

excluded = sorted(set(df["label"]) - set(realistic["label"]))
print(f"loaded {len(df)} runs; {len(realistic)} realistic (pool_share<{SHARE_CAP})")
if excluded:
    print(f"pools excluded from quantitative charts (thin / over-shared): {excluded}")


def exp1_fee_vs_il():
    """Headline: fee income vs impermanent loss vs net, per pool, hold @ ±10%."""
    d = realistic[(realistic.range_width == HEADLINE_WIDTH) & (~realistic.rebalance)]
    d = d.sort_values("net_pct", ascending=False)
    if d.empty:
        print("exp1: no realistic rows at headline width; skipping")
        return
    fig, ax = plt.subplots(figsize=(9, 5))
    x = range(len(d))
    ax.bar([i - 0.25 for i in x], d.fees_pct, 0.25, label="Fees", color="#2e7d32")
    ax.bar([i for i in x], d.il_pct, 0.25, label="Impermanent loss", color="#c62828")
    ax.bar([i + 0.25 for i in x], d.net_pct, 0.25, label="Net P&L", color="#1565c0")
    ax.set_xticks(list(x))
    ax.set_xticklabels(d.label, rotation=20, ha="right")
    ax.axhline(0, color="black", lw=0.8)
    ax.set_ylabel("% of capital (period)")
    ax.set_title(f"Fee income dwarfs IL — hold, ±{int(HEADLINE_WIDTH*100)}% range")
    ax.legend()
    fig.tight_layout()
    fig.savefig(os.path.join(CHARTS, "exp1_fee_vs_il.png"), dpi=120)
    plt.close(fig)
    print("wrote exp1_fee_vs_il.png")


def exp2_range_width():
    """Range-width knob on the focus pool (hold): net%, fee APY, days-in-range."""
    d = df[(df.label == FOCUS_POOL) & (~df.rebalance)].sort_values("range_width")
    if d.empty:
        print(f"exp2: no rows for {FOCUS_POOL}; skipping")
        return
    fig, ax1 = plt.subplots(figsize=(9, 5))
    w = d.range_width * 100
    ax1.plot(w, d.net_pct, "o-", color="#1565c0", label="Net P&L %")
    ax1.plot(w, d.fee_apy_pct, "s--", color="#2e7d32", label="Fee APY %")
    ax1.set_xlabel("Range half-width (±%)")
    ax1.set_ylabel("Net P&L % / Fee APY %")
    ax2 = ax1.twinx()
    ax2.plot(w, d.days_in_range_pct, "^:", color="#ef6c00", label="Days in range %")
    ax2.set_ylabel("Days in range %")
    ax2.set_ylim(0, 105)
    lines = ax1.get_lines() + ax2.get_lines()
    ax1.legend(lines, [l.get_label() for l in lines], loc="center right")
    ax1.set_title(f"{FOCUS_POOL}: narrower range lifts modelled net but collapses time-in-range (hold)")
    fig.tight_layout()
    fig.savefig(os.path.join(CHARTS, "exp2_range_width.png"), dpi=120)
    plt.close(fig)
    print("wrote exp2_range_width.png")


def exp3_rebalance_cost():
    """Rebalancing to stay in range locks in IL — net %, hold vs rebalance @±10%.

    NOTE the fee model is range-independent (T-03-09), so fees are identical hold
    vs rebalance here: the chart isolates the IL *cost* of rebalancing but not the
    fee *benefit* a real rebalance earns by staying in range. Read as an upper
    bound on the harm, not a verdict.
    """
    d = df[(df.range_width == HEADLINE_WIDTH) & (df.pool_share < SHARE_CAP)]
    pools = sorted(d.label.unique(), key=lambda p: d[d.label == p].realized_vol.iloc[0])
    if not pools:
        print("exp3: no rows; skipping")
        return
    hold = [d[(d.label == p) & (~d.rebalance)].net_pct.iloc[0] for p in pools]
    reb = [d[(d.label == p) & (d.rebalance)].net_pct.iloc[0] for p in pools]
    fig, ax = plt.subplots(figsize=(9, 5))
    x = range(len(pools))
    ax.bar([i - 0.2 for i in x], hold, 0.4, label="Hold", color="#1565c0")
    ax.bar([i + 0.2 for i in x], reb, 0.4, label="Rebalance on exit", color="#c62828")
    ax.set_xticks(list(x))
    ax.set_xticklabels(pools, rotation=20, ha="right")
    ax.axhline(0, color="black", lw=0.8)
    ax.set_ylabel("Net P&L % of capital (period)")
    ax.set_title("Rebalancing locks in IL and cuts net return (±10%, IL cost only)")
    ax.legend()
    fig.tight_layout()
    fig.savefig(os.path.join(CHARTS, "exp3_rebalance_cost.png"), dpi=120)
    plt.close(fig)
    print("wrote exp3_rebalance_cost.png")


if __name__ == "__main__":
    exp1_fee_vs_il()
    exp2_range_width()
    exp3_rebalance_cost()
    print("done")
