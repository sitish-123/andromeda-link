#!/usr/bin/env python3
"""Writes the Flatpak sources that let cargo build offline: every crates.io package in Cargo.lock, unpacked into
cargo/vendor with the checksum file cargo checks, and a config that points cargo at that directory.

Usage: flatpak-cargo-sources.py Cargo.lock OUT.json
"""
import json
import sys
import tomllib

REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"


def main(lock_path: str, out_path: str) -> None:
    with open(lock_path, "rb") as lock:
        packages = tomllib.load(lock)["package"]
    sources = []
    for package in packages:
        source = package.get("source")
        if source is None:
            continue
        if source != REGISTRY:
            sys.exit(f"{package['name']}: only crates.io sources are supported, not {source}")
        name, version, checksum = package["name"], package["version"], package["checksum"]
        dest = f"cargo/vendor/{name}-{version}"
        sources.append({
            "type": "archive",
            "archive-type": "tar-gzip",
            "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
            "sha256": checksum,
            "dest": dest,
        })
        sources.append({
            "type": "inline",
            "contents": json.dumps({"package": checksum, "files": {}}),
            "dest": dest,
            "dest-filename": ".cargo-checksum.json",
        })
    sources.append({
        "type": "inline",
        "contents": '[source.vendored-sources]\ndirectory = "cargo/vendor"\n\n'
        '[source.crates-io]\nreplace-with = "vendored-sources"\n',
        "dest": "cargo",
        "dest-filename": "config.toml",
    })
    with open(out_path, "w") as out:
        json.dump(sources, out, indent=1)
        out.write("\n")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
