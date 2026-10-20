"""Run real mint regression tests only against the exact stage 00 CDK source pin."""
import argparse
import os
from pathlib import Path
import subprocess

REVISION = "9f6d3a3734d7223d10b5553e3591d3f9d5c6c904"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, required=True)
    parser.add_argument("--protoc", type=Path, required=True)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    source = args.source.resolve()
    revision = subprocess.check_output(["git", "-C", str(source), "rev-parse", "HEAD"], text=True).strip()
    if revision != REVISION:
        raise SystemExit("refusing unpinned CDK revision")
    if subprocess.check_output(["git", "-C", str(source), "status", "--porcelain", "--untracked-files=no"], text=True).strip():
        raise SystemExit("refusing modified CDK source/lockfile")
    environment = dict(os.environ, PROTOC=str(args.protoc.resolve()), CARGO_BUILD_JOBS="2")
    base = ["cargo", "+stable", "test", "--manifest-path", str(source / "Cargo.toml"), "--locked"]
    if args.offline:
        base.append("--offline")
    for arguments in [
        ["-p", "cashu", "--no-default-features", "--features", "mint", "--lib", "nuts::nut00::tests"],
        ["-p", "cdk", "--no-default-features", "--features", "mint", "--lib", "pending_async_melt_tests"],
        ["-p", "cdk", "--no-default-features", "--features", "mint", "--lib", "mint::saga_recovery::tests"],
    ]:
        subprocess.run(base + arguments + ["--", "--test-threads=1"], env=environment, check=True)


if __name__ == "__main__":
    main()
