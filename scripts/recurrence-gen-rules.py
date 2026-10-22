#!/usr/bin/env python3
"""Generate recurrence rules (one JSON object per line) for the Rust/Go
differential check. Usage: recurrence-gen-rules.py <seed> <count>

Every BY-part is built to include the chosen start, so most rules are
synchronized; the implementations, not this script, decide acceptance.
Deterministic for a given seed. Expected results are never written here.
"""

import json
import random
import sys
import datetime as dt

rnd = random.Random(int(sys.argv[1]))
n = int(sys.argv[2])
zones = [
    "America/New_York",
    "Europe/Berlin",
    "Australia/Lord_Howe",
    "Asia/Tokyo",
    "Etc/UTC",
    "America/Winnipeg",
    "Europe/Oslo",
    "Pacific/Chatham",
    "America/St_Johns",
    "Asia/Kolkata",
    "America/Sao_Paulo",
    "Europe/London",
]
freqs = ["MINUTELY", "HOURLY", "DAILY", "WEEKLY", "MONTHLY", "YEARLY"]
days = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"]


def maybe(p):
    return rnd.random() < p


for _ in range(n):
    f = rnd.choice(freqs)
    r = {"tzid": rnd.choice(zones), "freq": f}
    if maybe(0.4):
        r["interval"] = rnd.choice([1, 2, 3, 5, 7])
    if maybe(0.5):
        r["count"] = rnd.randint(1, 30)
    elif maybe(0.3):
        r["until"] = (
            f"{rnd.choice([2026, 2027]):04d}-{rnd.randint(1, 12):02d}-{rnd.randint(1, 28):02d}T{rnd.randint(0, 23):02d}:00:00Z"
        )
    if maybe(0.15):
        r["wkst"] = rnd.choice(days)
    # start: a concrete date, then make every BY-part include it
    d = dt.date(2026, 1, 1) + dt.timedelta(days=rnd.randint(-200, 500))
    h = rnd.choice([0, 1, 2, 2, 3, 9, 12, 23])
    mi = rnd.choice([0, 15, 30, 45, 59])
    if maybe(0.4):
        r["by_day"] = [{"day": days[d.weekday()]}] + [
            {"day": rnd.choice(days)} for _ in range(rnd.randint(0, 2))
        ]
        if f in ("MONTHLY", "YEARLY") and maybe(0.4):
            r["by_day"].append(
                {"day": rnd.choice(days), "ordinal": rnd.choice([1, 2, -1, 3, 6])}
            )
    if maybe(0.3):
        r["by_month"] = sorted(
            {d.month} | set(rnd.sample(range(1, 13), rnd.randint(0, 3)))
        )
    if f != "WEEKLY" and maybe(0.3):
        r["by_month_day"] = sorted(
            {d.day} | set(rnd.sample([1, 15, 28, 29, 30, 31, -1], rnd.randint(0, 2)))
        )
    if maybe(0.3):
        r["by_hour"] = sorted(
            {h} | set(rnd.sample([0, 1, 2, 3, 9, 12, 23], rnd.randint(0, 2)))
        )
    if maybe(0.3):
        r["by_minute"] = sorted(
            {mi} | set(rnd.sample([0, 15, 30, 45, 59], rnd.randint(0, 2)))
        )
    if maybe(0.15) and any(
        k in r for k in ("by_day", "by_month", "by_month_day", "by_hour", "by_minute")
    ):
        r["by_set_pos"] = [rnd.choice([1, -1, 2])]
    if maybe(0.2):
        r["rdate"] = [
            (
                dt.datetime(2026, 1, 1) + dt.timedelta(hours=rnd.randint(0, 9000))
            ).strftime("%Y-%m-%dT%H:30:00")
            for _ in range(2)
        ]
    if maybe(0.2):
        r["exdate"] = [
            (
                dt.datetime(2026, 1, 1) + dt.timedelta(hours=rnd.randint(0, 9000))
            ).strftime("%Y-%m-%dT%H:30:00")
            for _ in range(2)
        ]
    r["dtstart"] = f"{d.year:04d}-{d.month:02d}-{d.day:02d}T{h:02d}:{mi:02d}:00"
    print(json.dumps(r))
