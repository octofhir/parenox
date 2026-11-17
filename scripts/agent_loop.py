#!/usr/bin/env python3
"""Exercise a persistent checker and measure actual subprocess/JSON round trips."""

import argparse
import json
import platform
import statistics
import subprocess
import time
from pathlib import Path


def summarize(samples):
    samples = sorted(samples)
    return {
        "p50_us": round(statistics.median(samples), 1),
        "p95_us": round(samples[(len(samples) * 95 + 99) // 100 - 1], 1),
        "max_us": round(samples[-1], 1),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", default="target/release/parenox")
    parser.add_argument("--iterations", type=int, default=100)
    parser.add_argument("--functions", type=int, default=1000)
    parser.add_argument("--project", help="Run the multi-namespace clinic scenario")
    args = parser.parse_args()
    if args.iterations < 1 or args.functions < 1:
        parser.error("iterations and functions must be positive")
    root = Path(__file__).resolve().parent.parent
    binary = (root / args.binary).resolve()
    process = subprocess.Popen(
        [str(binary), "serve"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        text=True,
        bufsize=1,
        cwd=root,
    )
    sequence = 0

    def send(payload):
        nonlocal sequence
        sequence += 1
        started = time.perf_counter_ns()
        process.stdin.write(json.dumps({"id": sequence, **payload}) + "\n")
        process.stdin.flush()
        line = process.stdout.readline()
        if not line:
            raise RuntimeError("Checker exited without a response")
        response = json.loads(line)
        elapsed_us = (time.perf_counter_ns() - started) / 1000
        assert response["id"] == sequence, response
        return response["result"], elapsed_us

    def request(path, source):
        return send({"path": path, "source": source})

    try:
        if args.project:
            project = root / args.project
            files = [{"path": str(p.relative_to(root)), "source": p.read_text()}
                     for p in sorted(project.rglob("*.clj"))]
            extern_path = project / "externs.edn"
            payload = {"files": files, "externs": {
                "path": str(extern_path.relative_to(root)), "source": extern_path.read_text(),
            }}
            initial, first_us = send(payload)
            errors = [{"file": f["path"], "line": d["span"]["start"]["line"],
                       "code": d["code"], "message": d["message"]}
                      for f in initial["files"] for d in f["diagnostics"]]
            assert len(errors) == 3 and initial["complete"], initial
            for f in files:
                f["source"] = f["source"].replace(
                    '(str/upper-case (:name patient))',
                    '(str/upper-case (or (:name patient) "Unknown"))'
                ).replace('(store/find-patient 42)', '(store/find-patient id)') \
                 .replace('(str/starts-with? 42', '(str/starts-with? id')
            fixed, fix_us = send(payload)
            assert fixed["ok"] and fixed["stats"]["checked_functions"] == 2, fixed
            warm_times, edit_times = [], []
            service = next(f for f in files if f["path"].endswith("service.clj"))
            for index in range(args.iterations):
                warm, elapsed = send(payload)
                assert warm["ok"] and warm["stats"]["checked_functions"] == 0, warm
                assert warm["stats"]["parsed_files"] == 0, warm
                warm_times.append(elapsed)
                old, new = ("Unknown", "Anonymous") if index % 2 == 0 else ("Anonymous", "Unknown")
                service["source"] = service["source"].replace(old, new)
                edited, elapsed = send(payload)
                assert edited["ok"] and edited["stats"]["checked_functions"] == 1, edited
                assert edited["stats"]["reused_functions"] == 2, edited
                edit_times.append(elapsed)
            service["source"] = service["source"].replace(
                "'[::model/patient :-> string?]", "'[::model/patient :-> number?]")
            changed, change_us = send(payload)
            assert not changed["ok"] and changed["stats"]["checked_functions"] == 2, changed
            assert any(not f["ok"] and f["path"].endswith("http.clj") for f in changed["files"]), changed
            print(json.dumps({
                "scenario": "clinic: 4 namespaces + 1 declared library contract",
                "platform": {"os": platform.system(), "arch": platform.machine()},
                "iterations": args.iterations,
                "initial_errors": errors,
                "first_round_trip_us": round(first_us, 1),
                "fix_round_trip_us": round(fix_us, 1),
                "after_fix": fixed["stats"],
                "warm_round_trip": summarize(warm_times),
                "body_edit_round_trip": summarize(edit_times),
                "body_edit": edited["stats"],
                "signature_change": {"round_trip_us": round(change_us, 1), **changed["stats"]},
                "scope": "Release process, Python JSON + pipes + full project snapshots + response decoding. Local example, not a production-scale benchmark.",
            }, indent=2))
            return
        bad = "(ns demo (:require [clojure.spec.alpha :as s] [clojure.string :as str]))\n" \
            "(defn label {:t/sig '[(s/nilable string?) :-> string?]} [name] (str/upper-case name))"
        report, _ = request("edit.clj", bad)
        assert not report["ok"] and report["complete"], report
        assert report["diagnostics"][0]["code"] == "PNX2322", report
        fixed = bad.replace("(str/upper-case name)", '(str/upper-case (or name "Unknown"))')
        report, _ = request("edit.clj", fixed)
        assert report["ok"] and report["stats"]["checked_functions"] == 1, report
        report, _ = request("edit.clj", fixed)
        assert report["ok"] and report["stats"]["reused_functions"] == 1, report

        small = (root / "examples/good.clj").read_text()
        report, small_first = request("small.clj", small)
        assert report["ok"], report
        small_times = []
        for _ in range(args.iterations):
            report, elapsed = request("small.clj", small)
            assert report["ok"] and report["stats"]["reused_functions"] == 4, report
            small_times.append(elapsed)

        large = "(ns generated)\n" + "\n".join(
            f"(defn f{i} {{:t/sig '[number? :-> number?]}} [x] (+ x 1))"
            for i in range(args.functions)
        )
        report, large_first = request("large.clj", large)
        assert report["ok"] and report["stats"]["checked_functions"] == args.functions, report
        large_times = []
        edited_times = []
        for index in range(args.iterations):
            report, elapsed = request("large.clj", large)
            assert report["ok"] and report["stats"]["reused_functions"] == args.functions, report
            large_times.append(elapsed)
            # Same-width body edit in the final function preserves all other source ranges.
            position = large.rfind("(+ x ")
            large = large[:position] + f"(+ x {2 if index % 2 == 0 else 1}))"
            report, elapsed = request("large.clj", large)
            assert report["ok"], report
            assert report["stats"]["checked_functions"] == 1, report
            assert report["stats"]["reused_functions"] == args.functions - 1, report
            edited_times.append(elapsed)

        cold_times = []
        for _ in range(min(args.iterations, 30)):
            started = time.perf_counter_ns()
            completed = subprocess.run(
                [str(binary), "check", "examples/good.clj", "--json"],
                cwd=root, capture_output=True, text=True, check=True,
            )
            assert json.loads(completed.stdout)["ok"]
            cold_times.append((time.perf_counter_ns() - started) / 1000)

        print(json.dumps({
            "platform": {"os": platform.system(), "arch": platform.machine()},
            "iterations": args.iterations,
            "agent_edit_fix_recheck": "passed",
            "small_file": {
                "bytes": len(small.encode()), "functions": 4,
                "first_round_trip_us": round(small_first, 1),
                "warm_round_trip": summarize(small_times),
            },
            "generated_file": {
                "bytes": len(large.encode()), "functions": args.functions,
                "first_round_trip_us": round(large_first, 1),
                "warm_round_trip": summarize(large_times),
                "single_body_edit_round_trip": summarize(edited_times),
            },
            "fresh_cli_process": {"iterations": len(cold_times), **summarize(cold_times)},
            "scope": "Synthetic single-file fixtures. Round trips include Python JSON encoding, pipes, full source transfer, parsing, checking and response decoding. Fresh CLI includes process startup and file I/O. No JVM or macro expansion.",
        }, indent=2))
    finally:
        process.stdin.close()
        process.wait(timeout=5)
        process.stdout.close()


if __name__ == "__main__":
    main()
