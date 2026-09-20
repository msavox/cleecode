#!/usr/bin/env python3
"""Reads one or more CLEE_GRAPHICS_PROFILE files and prints the table the decision needs.

    python3 scripts/profile_digest.py /tmp/clee-video-profile/profile-A-shmyes-*.json

Several files of the same shape are averaged and their spread is shown, because a number that
moves by a factor of two between two runs of the same thing is not a number.
"""

import json
import statistics
import sys

# Which bucket each stage falls in, for the question the profile exists to answer: what would
# the shared-memory relay actually remove?
#
#   gone    — the relay does not do this at all. The resample, the RGBA conversion and the
#             base64 all live inside `render`; `stdout_write` is the escape sequence reaching
#             the host, which becomes some four hundred bytes.
#   less    — the relay still does this, but once instead of twice, or on a smaller thing.
#   same    — the relay changes nothing here.
BUCKET = {
    "render": "gone",
    "stdout_write": "gone",
    "shm_read": "less",
    "clone": "less",
    "protocol_build": "less",
    "splitter": "same",
    "base64_decode": "same",
    "wrap": "same",
    "stdout_flush": "same",
}


def digest(paths):
    runs = [json.load(open(path)) for path in paths]
    print(f"{len(runs)} run(s): {', '.join(paths)}")
    spread = lambda values: (
        f"{statistics.mean(values):,.1f}"
        if max(values) - min(values) <= 0.05 * max(max(values), 1e-9)
        else f"{min(values):,.1f}-{max(values):,.1f}"
    )

    for key, label in [
        ("wall_seconds", "wall seconds"),
        ("cpu_total_seconds", "clee CPU seconds"),
        ("instrumented_percent_of_cpu", "accounted for by the stages, %"),
        ("pictures_decoded", "pictures decoded"),
        ("pictures_paced_out", "pictures Pace refused"),
        ("editor_frames", "editor frames drawn"),
        ("span_overhead_ns", "cost of one measured span, ns"),
    ]:
        print(f"  {label:<34} {spread([float(run[key]) for run in runs])}")
    picture = [run["frames_with_picture"] for run in runs]
    plain = [run["frames_without_picture"] for run in runs]
    print(f"  {'stdout bytes, frame with a picture':<34} {spread([f['mean_bytes'] for f in picture])}")
    print(f"  {'stdout bytes, frame without one':<34} {spread([f['mean_bytes'] for f in plain])}")

    print()
    # The columns are measured before they are printed rather than given widths chosen in
    # advance. A spread reads `1,340.6-1,419.8` when two runs disagree — fifteen characters,
    # where a guessed thirteen had the number run into its neighbour with no space between
    # them, so that `397.0` and `1,191.6` arrived as `397.01,191.6` and the reader had to take
    # the table apart by eye to find out which was which.
    headings = ("stage", "bucket", "CPU ms", "% of CPU", "calls", "CPU us/call", "p95 wall us")
    rows = []
    buckets = {"gone": [], "less": [], "same": []}
    for index in range(len(runs[0]["stages"])):
        stages = [run["stages"][index] for run in runs]
        name = stages[0]["stage"]
        percent = [s["percent_of_process_cpu"] for s in stages]
        buckets[BUCKET[name]].append(statistics.mean(percent))
        rows.append((
            name,
            BUCKET[name],
            spread([s["cpu_ns"] / 1e6 for s in stages]),
            spread(percent),
            spread([float(s["calls"]) for s in stages]),
            spread([s["cpu_mean_ns"] / 1e3 for s in stages]),
            spread([s["wall_p95_ns"] / 1e3 for s in stages]),
        ))
    widths = [max(len(cell) for cell in column) for column in zip(headings, *rows)]
    # The two names on the left read as names and the five numbers on the right as numbers.
    line = lambda cells: "  " + " ".join(
        f"{cell:<{width}}" if index < 2 else f"{cell:>{width}}"
        for index, (cell, width) in enumerate(zip(cells, widths))
    )
    print(line(headings))
    for row in rows:
        print(line(row))
    accounted = statistics.mean([run["instrumented_percent_of_cpu"] for run in runs])
    print()
    print(f"  the relay would remove        {sum(buckets['gone']):5.1f} % of clee's CPU")
    print(f"  the relay would only reduce   {sum(buckets['less']):5.1f} %")
    print(f"  the relay would not touch     {sum(buckets['same']):5.1f} %")
    print(f"  not accounted for at all      {100 - accounted:5.1f} %")
    print()
    for run in runs[:1]:
        for fit in run["fits"]:
            print(
                f"  source {fit['src'][0]}x{fit['src'][1]} into {fit['dst'][0]}x{fit['dst'][1]} "
                f"pixels, {fit['count']} frames, same size: {fit['same']}"
            )
        for arrival in run["arrivals"]:
            print(f"  arrived as s={arrival['s']} v={arrival['v']} f={arrival['f']}, {arrival['count']} frames")


if __name__ == "__main__":
    digest(sys.argv[1:])
