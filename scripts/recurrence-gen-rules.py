#!/usr/bin/env python3
"""Generate recurrence rules (one JSON object per line) for the Rust/Go
differential check. Usage: recurrence-gen-rules.py <seed> <count>

Every BY-part is built to include the chosen start, so most rules are
synchronized; the implementations, not this script, decide acceptance.
About half of the starts lie within 150 minutes of a real UTC-offset
transition read from the committed tzdb bundle, so gaps and folds are
exercised. Deterministic for a given seed. Expected results are never
written here.
"""

import datetime as dt
import json
import pathlib
import random
import struct
import sys

rnd = random.Random(int(sys.argv[1]))
n = int(sys.argv[2])

BUNDLE = (
    pathlib.Path(__file__).resolve().parent.parent
    / "crates/sysprims-recurrence/tzdata/tzif-bundle-2026e.bin"
)
WINDOW = (
    int(dt.datetime(2025, 1, 1, tzinfo=dt.timezone.utc).timestamp()),
    int(dt.datetime(2028, 1, 1, tzinfo=dt.timezone.utc).timestamp()),
)


def read_bundle(path):
    raw = path.read_bytes()
    split = raw.index(b"\n\n")
    lines = raw[:split].decode("ascii").split("\n")
    data = raw[split + 2 :]
    index = {}
    for line in lines[3:]:
        name, off, size = line.split(" ")
        index[name] = data[int(off) : int(off) + int(size)]
    return index


def tzif_transitions(blob):
    """(utc_seconds, offset_before, offset_after) from a TZif v2+ blob's
    64-bit section (RFC 8536)."""

    def header(at):
        if blob[at : at + 4] != b"TZif":
            raise ValueError("not TZif")
        return struct.unpack(">6l", blob[at + 20 : at + 44])

    isutc, isstd, leap, timecnt, typecnt, charcnt = header(0)
    v1 = timecnt * 5 + typecnt * 6 + charcnt + leap * 8 + isstd + isutc
    at = 44 + v1
    isutc, isstd, leap, timecnt, typecnt, charcnt = header(at)
    at += 44
    times = struct.unpack(f">{timecnt}q", blob[at : at + 8 * timecnt])
    at += 8 * timecnt
    idx = blob[at : at + timecnt]
    at += timecnt
    offsets = [
        struct.unpack(">lBB", blob[at + 6 * i : at + 6 * i + 6])[0]
        for i in range(typecnt)
    ]
    out = []
    for i, t in enumerate(times):
        before = offsets[idx[i - 1]] if i else offsets[0]
        after = offsets[idx[i]]
        if before != after:
            out.append((t, before, after))
    return out


ZONE_BLOBS = read_bundle(BUNDLE)
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
TRANSITIONS = {
    z: [t for t in tzif_transitions(ZONE_BLOBS[z]) if WINDOW[0] <= t[0] < WINDOW[1]]
    for z in zones
}
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
    # start: a concrete local date-time, then make every BY-part include it
    near = TRANSITIONS[r["tzid"]]
    if near and maybe(0.5):
        # Within 150 minutes of a real transition, labelled with either side's
        # offset so labels land in gaps, folds and their edges.
        t, before, after = rnd.choice(near)
        utc = t + 60 * rnd.randint(-150, 150)
        local = dt.datetime(1970, 1, 1) + dt.timedelta(
            seconds=utc + rnd.choice([before, after])
        )
        d = local.date()
        h, mi = local.hour, local.minute
    else:
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
