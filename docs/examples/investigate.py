import argparse
import json
import math
import subprocess
import time
from decimal import Decimal
from itertools import count


def page_items(fetch, items, next_key):
    cursor = None
    while True:
        page = fetch(cursor)
        yield from page[items]
        cursor = page[next_key]
        if cursor is None:
            return


def poll(fetch, is_pending, *, initial=None):
    if initial is not None and not is_pending(initial):
        return initial
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        reply = fetch()
        if not is_pending(reply):
            return reply
        time.sleep(0.1)
    raise RuntimeError("Timed out waiting for GeoTrace")


def choose_demo(entries, *, key, value):
    demos = [entry for entry in entries if entry[key] == value]
    if not demos:
        raise RuntimeError("Open or store the demo recording before running this example")
    if len(demos) > 1:
        raise RuntimeError("Multiple demo recordings match. Use a session with one demo recording.")
    return demos[0]


def investigate(call, workflow, requested_mask):
    if workflow == "history":
        dates = {"from": "2026-02-01", "through": "2026-02-01"}

        def history_page(cursor):
            def fetch():
                try:
                    return call("list_history_recordings", dates=dates, cursor=cursor)
                except RuntimeError as error:
                    if str(error) != "GeoTrace is preparing recording History. Try again.":
                        raise
                    return None

            return poll(fetch, lambda reply: reply is None)

        stored = page_items(history_page, "recordings", "next_cursor")
        demo = choose_demo(stored, key="identity", value="demo-trip-v1")
        opening = call("open_history_recording", recording=demo["id"])
        opened = poll(
            lambda: call("get_history_open_status", operation=opening["operation"]),
            lambda reply: reply["state"]["kind"] in {"reading", "loading"},
            initial=opening,
        )["state"]
        if opened["kind"] != "ready":
            raise RuntimeError(opened)
        recording = opened["recording"]
    else:
        loaded = page_items(
            lambda after: call("list_recordings", after=after), "recordings", "next_after",
        )
        demo = choose_demo(loaded, key="filename", value="demo_trip.gtd")
        recording = demo["id"]

    tracks = page_items(
        lambda after: call("get_recording", recording=recording, after=after),
        "tracks", "next_after",
    )
    details = {track["index"]: call("get_track", track=track) for track in tracks}
    print("Tracks:", details)

    metrics = ["eph", "sats_seen", "sats_fix"]
    if workflow == "history":
        metrics.append("util_all")
    definitions = call("list_metrics")["definitions"]
    print("Metrics:", [metric for metric in definitions if metric["name"] in metrics])

    def read_matches(mask=None):
        source = "points"
        if mask is not None:
            mask_literal = format(Decimal(str(mask)), "f")
            source = f"points | with mask {mask_literal} deg"
        text = f"{source} | where eph > 20 m | table {', '.join(metrics)}"
        checked = call("check_query", text=text)
        if checked["status"] != "valid":
            raise RuntimeError(checked)
        started = call("run_query", text=text)
        if "job" not in started or started["status"] == "busy":
            raise RuntimeError(started)
        job = started["job"]
        finished = poll(
            lambda: call("get_query_status", job=job),
            lambda reply: reply["status"] in {"preparing", "running", "cancelling"},
            initial=started,
        )
        if finished["status"] != "completed":
            raise RuntimeError(finished)

        matches = page_items(
            lambda start: call(
                "get_query_results", job=job, query=0, limit=1,
                page={"kind": "matches", "start": start},
            )["page"],
            "matches", "next_start",
        )
        match = next((item for item in matches if item["track"]["recording"] == recording), None)
        if match is None:
            raise RuntimeError("No visible demo points with EPH above 20 m")

        rows = {}
        start = None
        while True:
            batch = call(
                "get_query_results", job=job, query=0, limit=1,
                page={"kind": "rows", "matched": match["position"], "start": start},
            )
            for row in batch["page"]["rows"]:
                rows[(row["index"], row["time"])] = {
                    column["label"]: cell.get("value")
                    for column, cell in zip(batch["columns"], row["values"], strict=True)
                    if column["label"] in metrics
                }
            start = batch["page"]["next_start"]
            if start is None:
                return job, match, rows

    setting = "analysis.elevation_mask_deg"
    mask = None
    if workflow == "history":
        settings = call("get_settings")["settings"]
        print("Analysis settings:", [item for item in settings if item["page"] == "Analysis"])
        mask = call("describe_setting", key=setting)["value"]

    job, match, rows = read_matches(mask)
    print("Match range:", match["range"])
    print("Matched readings:", rows)
    track = match["track"]
    bounds = match["range"]
    start = max(0, bounds["start"] - 2)
    end = min(details[track["index"]]["point_count"], bounds["end"] + 2)
    surrounding = page_items(
        lambda cursor: call(
            "get_range_details", track=track, start=start if cursor is None else cursor,
            end=end, limit=4,
        ),
        "points", "next_start",
    )
    print("Surrounding points:", list(surrounding))

    if requested_mask is not None:
        try:
            effective = call("set_setting", key=setting, value=requested_mask)["value"]
            job, match, updated = read_matches(effective)
            for point in rows:
                if point in updated:
                    print(point, mask, rows[point], effective, updated[point])
        finally:
            call("set_setting", key=setting, value=mask)

    target = {
        "kind": "query_match", "job": job, "query": 0, "matched": match["position"],
    }
    print(call("show_on_map", target=target))
    print(call("show_on_plot", target=target, metrics=metrics))
    if workflow == "history":
        print(call("open_setting", key=setting))


parser = argparse.ArgumentParser()
parser.add_argument("workflow", choices=["loaded", "history"])
parser.add_argument("--companion", default="geotrace-mcp")
parser.add_argument("--elevation-mask", type=float)
options = parser.parse_args()
if options.workflow != "history" and options.elevation_mask is not None:
    parser.error("--elevation-mask requires the history workflow")
if options.elevation_mask is not None and not math.isfinite(options.elevation_mask):
    parser.error("--elevation-mask requires a finite number")

with subprocess.Popen(
    [options.companion], stdin=subprocess.PIPE, stdout=subprocess.PIPE, encoding="utf-8",
) as server:
    request_ids = count(1)

    def rpc(method, params, notification=False):
        request = {"jsonrpc": "2.0", "method": method, "params": params}
        if not notification:
            request["id"] = next(request_ids)
        server.stdin.write(json.dumps(request, allow_nan=False) + "\n")
        server.stdin.flush()
        if not notification:
            reply = json.loads(server.stdout.readline())
            if reply.get("id") != request["id"] or "error" in reply:
                raise RuntimeError(reply)
            return reply["result"]

    rpc("initialize", {
        "protocolVersion": "2025-11-25", "capabilities": {},
        "clientInfo": {"name": "geotrace-investigation", "version": "1"},
    })
    rpc("notifications/initialized", {}, notification=True)

    def call(name, **arguments):
        result = rpc("tools/call", {"name": name, "arguments": arguments})
        if result.get("isError"):
            raise RuntimeError(result["content"][0]["text"])
        return result["structuredContent"]

    investigate(call, options.workflow, options.elevation_mask)
