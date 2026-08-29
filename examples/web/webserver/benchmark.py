#!/usr/bin/env python3
"""Dependency-free load tester for the Olang notes server.

Start the server in one terminal:

    olang main.ol 8080

Then run this script from another terminal:

    python3 benchmark.py
    python3 benchmark.py --duration 30 --concurrency 25

The default target, GET /notes, exercises routing, synchronized SQLite access,
JSON encoding, and the concurrent HTTP worker pool. Connections close after
every request by default so the run measures accept/dispatch costs and spreads
work evenly across workers. Pass --keep-alive to measure persistent HTTP/1.1.
"""

from __future__ import annotations

import argparse
import http.client
import math
import statistics
import sys
import threading
import time
from collections import Counter
from dataclasses import dataclass, field
from typing import Dict, Iterable, List, Optional, Tuple
from urllib.parse import urlsplit


@dataclass(frozen=True)
class Target:
    scheme: str
    host: str
    port: int
    path: str


@dataclass
class WorkerResult:
    latencies_ms: List[float] = field(default_factory=list)
    statuses: Counter = field(default_factory=Counter)
    errors: Counter = field(default_factory=Counter)
    error_samples: List[str] = field(default_factory=list)
    response_bytes: int = 0


@dataclass
class RunResult:
    elapsed: float
    latencies_ms: List[float]
    statuses: Counter
    errors: Counter
    error_samples: List[str]
    response_bytes: int


def positive_int(value: str) -> int:
    parsed = int(value)
    if parsed < 1:
        raise argparse.ArgumentTypeError("must be at least 1")
    return parsed


def positive_float(value: str) -> float:
    parsed = float(value)
    if parsed <= 0:
        raise argparse.ArgumentTypeError("must be greater than 0")
    return parsed


def non_negative_float(value: str) -> float:
    parsed = float(value)
    if parsed < 0:
        raise argparse.ArgumentTypeError("must be 0 or greater")
    return parsed


def header(value: str) -> Tuple[str, str]:
    if ":" not in value:
        raise argparse.ArgumentTypeError("must have the form 'Name: value'")
    name, raw_value = value.split(":", 1)
    if not name.strip():
        raise argparse.ArgumentTypeError("header name cannot be empty")
    return name.strip(), raw_value.strip()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Stress-test a running Olang webserver and print latency/throughput metrics."
    )
    parser.add_argument(
        "url",
        nargs="?",
        default="http://127.0.0.1:8080/notes",
        help="target URL (default: %(default)s)",
    )
    parser.add_argument(
        "-d",
        "--duration",
        type=positive_float,
        default=10.0,
        help="measured run time in seconds (default: %(default)s)",
    )
    parser.add_argument(
        "-c",
        "--concurrency",
        type=positive_int,
        default=10,
        help="number of client threads (default: %(default)s)",
    )
    parser.add_argument(
        "--warmup",
        type=non_negative_float,
        default=1.0,
        help="unmeasured warmup time in seconds (default: %(default)s)",
    )
    parser.add_argument(
        "--timeout",
        type=positive_float,
        default=5.0,
        help="per-request socket timeout in seconds (default: %(default)s)",
    )
    parser.add_argument(
        "--method",
        default="GET",
        help="HTTP method (default: %(default)s)",
    )
    parser.add_argument(
        "--body",
        default=None,
        help="request body; useful with --method POST",
    )
    parser.add_argument(
        "-H",
        "--header",
        dest="headers",
        action="append",
        type=header,
        default=[],
        metavar="'NAME: VALUE'",
        help="additional request header; may be repeated",
    )
    parser.add_argument(
        "--keep-alive",
        action="store_true",
        help="reuse one persistent connection per worker",
    )
    return parser.parse_args()


def parse_target(raw_url: str) -> Target:
    parsed = urlsplit(raw_url)
    if parsed.scheme not in ("http", "https"):
        raise ValueError("URL scheme must be http or https")
    if not parsed.hostname:
        raise ValueError("URL must include a host")
    try:
        port = parsed.port or (443 if parsed.scheme == "https" else 80)
    except ValueError as exc:
        raise ValueError("URL contains an invalid port") from exc
    path = parsed.path or "/"
    if parsed.query:
        path += "?" + parsed.query
    return Target(parsed.scheme, parsed.hostname, port, path)


def make_connection(target: Target, timeout: float) -> http.client.HTTPConnection:
    connection_type = (
        http.client.HTTPSConnection if target.scheme == "https" else http.client.HTTPConnection
    )
    return connection_type(target.host, target.port, timeout=timeout)


def make_headers(
    supplied: Iterable[Tuple[str, str]], keep_alive: bool, body: Optional[str]
) -> Dict[str, str]:
    headers = {name: value for name, value in supplied}
    headers["Connection"] = "keep-alive" if keep_alive else "close"
    headers.setdefault("User-Agent", "olang-benchmark/1")
    if body is not None:
        headers.setdefault("Content-Type", "application/json")
    return headers


def perform_request(
    connection: http.client.HTTPConnection,
    target: Target,
    method: str,
    body: Optional[str],
    headers: Dict[str, str],
) -> Tuple[int, int, bool]:
    connection.request(method, target.path, body=body, headers=headers)
    response = connection.getresponse()
    payload = response.read()
    server_closes = response.getheader("Connection", "").lower() == "close"
    return response.status, len(payload), server_closes


def worker(
    target: Target,
    method: str,
    body: Optional[str],
    headers: Dict[str, str],
    timeout: float,
    keep_alive: bool,
    start_event: threading.Event,
    stop_event: threading.Event,
    ready: threading.Condition,
    ready_count: List[int],
    deadline: List[float],
    result: WorkerResult,
) -> None:
    with ready:
        ready_count[0] += 1
        ready.notify_all()
    start_event.wait()

    connection: Optional[http.client.HTTPConnection] = None
    try:
        while not stop_event.is_set() and time.perf_counter() < deadline[0]:
            if connection is None:
                connection = make_connection(target, timeout)

            started = time.perf_counter()
            try:
                status, size, server_closes = perform_request(
                    connection, target, method, body, headers
                )
                result.latencies_ms.append((time.perf_counter() - started) * 1000.0)
                result.statuses[status] += 1
                result.response_bytes += size
                if not keep_alive or server_closes:
                    connection.close()
                    connection = None
            except Exception as exc:  # The exception is data for a load test.
                result.latencies_ms.append((time.perf_counter() - started) * 1000.0)
                error_name = type(exc).__name__
                result.errors[error_name] += 1
                if len(result.error_samples) < 3:
                    result.error_samples.append(f"{error_name}: {exc}")
                if connection is not None:
                    connection.close()
                    connection = None
    finally:
        if connection is not None:
            connection.close()


def run_load(
    target: Target,
    method: str,
    body: Optional[str],
    headers: Dict[str, str],
    duration: float,
    concurrency: int,
    timeout: float,
    keep_alive: bool,
) -> RunResult:
    start_event = threading.Event()
    stop_event = threading.Event()
    ready = threading.Condition()
    ready_count = [0]
    deadline = [0.0]
    worker_results = [WorkerResult() for _ in range(concurrency)]
    threads = [
        threading.Thread(
            target=worker,
            name=f"load-worker-{index + 1}",
            args=(
                target,
                method,
                body,
                headers,
                timeout,
                keep_alive,
                start_event,
                stop_event,
                ready,
                ready_count,
                deadline,
                worker_results[index],
            ),
            daemon=True,
        )
        for index in range(concurrency)
    ]

    for thread in threads:
        thread.start()
    with ready:
        ready.wait_for(lambda: ready_count[0] == concurrency)

    started = time.perf_counter()
    deadline[0] = started + duration
    start_event.set()
    try:
        for thread in threads:
            thread.join()
    except KeyboardInterrupt:
        stop_event.set()
        for thread in threads:
            thread.join(timeout + 0.5)
        raise
    elapsed = time.perf_counter() - started

    latencies: List[float] = []
    statuses: Counter = Counter()
    errors: Counter = Counter()
    samples: List[str] = []
    response_bytes = 0
    for item in worker_results:
        latencies.extend(item.latencies_ms)
        statuses.update(item.statuses)
        errors.update(item.errors)
        response_bytes += item.response_bytes
        for sample in item.error_samples:
            if sample not in samples and len(samples) < 5:
                samples.append(sample)

    return RunResult(elapsed, latencies, statuses, errors, samples, response_bytes)


def percentile(sorted_values: List[float], percent: float) -> float:
    if not sorted_values:
        return math.nan
    position = (len(sorted_values) - 1) * percent / 100.0
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return sorted_values[lower]
    weight = position - lower
    return sorted_values[lower] * (1.0 - weight) + sorted_values[upper] * weight


def format_bytes(value: float) -> str:
    units = ("B", "KiB", "MiB", "GiB")
    for unit in units[:-1]:
        if value < 1024.0:
            return f"{value:.2f} {unit}"
        value /= 1024.0
    return f"{value:.2f} {units[-1]}"


def print_report(result: RunResult, duration: float) -> int:
    completed = sum(result.statuses.values())
    transport_errors = sum(result.errors.values())
    attempted = completed + transport_errors
    successful = sum(count for status, count in result.statuses.items() if status < 400)
    http_errors = completed - successful
    sorted_latencies = sorted(result.latencies_ms)
    rate = completed / result.elapsed if result.elapsed else 0.0
    attempted_rate = attempted / result.elapsed if result.elapsed else 0.0
    byte_rate = result.response_bytes / result.elapsed if result.elapsed else 0.0
    mean_latency = statistics.fmean(sorted_latencies) if sorted_latencies else math.nan
    average_in_flight = (
        attempted_rate * mean_latency / 1000.0 if sorted_latencies else math.nan
    )

    print("\nResults")
    print("=" * 64)
    print(f"Wall time             {result.elapsed:12.3f} s (requested {duration:g} s)")
    print(f"Requests attempted    {attempted:12,d}")
    print(f"Responses completed   {completed:12,d}")
    print(f"Successful (<400)     {successful:12,d}")
    print(f"HTTP errors (>=400)   {http_errors:12,d}")
    print(f"Transport errors      {transport_errors:12,d}")
    print(f"Response throughput   {rate:12.2f} req/s")
    print(f"Attempt throughput    {attempted_rate:12.2f} req/s")
    if sorted_latencies:
        print(f"Average in flight     {average_in_flight:12.2f}")
    print(f"Response data         {format_bytes(result.response_bytes):>16}")
    print(f"Data throughput       {format_bytes(byte_rate) + '/s':>16}")

    if sorted_latencies:
        print("\nLatency (includes failed transport attempts)")
        print("-" * 64)
        rows = (
            ("minimum", sorted_latencies[0]),
            ("mean", mean_latency),
            ("p50", percentile(sorted_latencies, 50)),
            ("p90", percentile(sorted_latencies, 90)),
            ("p95", percentile(sorted_latencies, 95)),
            ("p99", percentile(sorted_latencies, 99)),
            ("maximum", sorted_latencies[-1]),
        )
        for label, value in rows:
            print(f"{label:>10} {value:12.3f} ms")

    if result.statuses:
        print("\nHTTP status distribution")
        print("-" * 64)
        for status, count in sorted(result.statuses.items()):
            percentage = count * 100.0 / completed
            print(f"{status:>10} {count:12,d}  {percentage:6.2f}%")

    if result.errors:
        print("\nTransport error distribution")
        print("-" * 64)
        for name, count in result.errors.most_common():
            print(f"{name:>20} {count:12,d}")
        if result.error_samples:
            print("\nSamples:")
            for sample in result.error_samples:
                print(f"  - {sample}")

    if attempted == 0:
        print("\nNo requests completed or failed; check the target and timeout.")
        return 2
    return 0 if http_errors == 0 and transport_errors == 0 else 1


def preflight(
    target: Target,
    method: str,
    body: Optional[str],
    headers: Dict[str, str],
    timeout: float,
) -> int:
    connection = make_connection(target, timeout)
    check_headers = dict(headers)
    check_headers["Connection"] = "close"
    try:
        status, _, _ = perform_request(connection, target, method, body, check_headers)
        return status
    finally:
        connection.close()


def main() -> int:
    args = parse_args()
    try:
        target = parse_target(args.url)
    except ValueError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2

    method = args.method.upper()
    headers = make_headers(args.headers, args.keep_alive, args.body)
    connection_mode = "keep-alive" if args.keep_alive else "close per request"

    print("Olang HTTP benchmark")
    print("=" * 64)
    print(f"Target       {args.url}")
    print(f"Request      {method} {target.path}")
    print(f"Duration     {args.duration:g} s (+ {args.warmup:g} s warmup)")
    print(f"Concurrency  {args.concurrency}")
    print(f"Connections  {connection_mode}")
    print("Server logs  keep OLANG_ACCESS_LOG=errors/off for clean performance measurements")

    try:
        status = preflight(target, method, args.body, headers, args.timeout)
    except Exception as exc:
        print(f"\nPreflight failed: {type(exc).__name__}: {exc}", file=sys.stderr)
        print("Is main.ol running at the target URL?", file=sys.stderr)
        return 2
    print(f"Preflight    HTTP {status}")

    try:
        if args.warmup:
            print("Warming up...", flush=True)
            run_load(
                target,
                method,
                args.body,
                headers,
                args.warmup,
                args.concurrency,
                args.timeout,
                args.keep_alive,
            )
        print("Benchmarking...", flush=True)
        result = run_load(
            target,
            method,
            args.body,
            headers,
            args.duration,
            args.concurrency,
            args.timeout,
            args.keep_alive,
        )
    except KeyboardInterrupt:
        print("\nBenchmark interrupted.", file=sys.stderr)
        return 130

    return print_report(result, args.duration)


if __name__ == "__main__":
    raise SystemExit(main())
