#!/usr/bin/env python3

import argparse
import json
from datetime import datetime
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Emit an Ophelia update manifest.")
    parser.add_argument("--channel", required=True, choices=("stable", "nightly"))
    parser.add_argument("--version", required=True)
    parser.add_argument("--pub-date", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--notes-url", required=True)
    parser.add_argument("--asset-url", required=True)
    parser.add_argument("--asset-size", required=True, type=int)
    parser.add_argument("--sha256", required=True)
    parser.add_argument("--minisign-url", required=True)
    parser.add_argument("--output", required=True)
    return parser.parse_args()


def require_non_empty(name: str, value: str) -> str:
    stripped = value.strip()
    if not stripped:
        raise SystemExit(f"{name} must not be empty")
    return stripped


def validate_rfc3339(value: str) -> str:
    normalized = require_non_empty("--pub-date", value)
    try:
        datetime.fromisoformat(normalized.replace("Z", "+00:00"))
    except ValueError as error:
        raise SystemExit(f"--pub-date must be RFC3339: {error}") from error
    return normalized


def main() -> None:
    args = parse_args()
    manifest = {
        "channel": args.channel,
        "version": require_non_empty("--version", args.version),
        "pub_date": validate_rfc3339(args.pub_date),
        "commit": require_non_empty("--commit", args.commit),
        "notes_url": require_non_empty("--notes-url", args.notes_url),
        "asset_url": require_non_empty("--asset-url", args.asset_url),
        "asset_size": args.asset_size,
        "sha256": require_non_empty("--sha256", args.sha256),
        "minisign_url": require_non_empty("--minisign-url", args.minisign_url),
    }

    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
