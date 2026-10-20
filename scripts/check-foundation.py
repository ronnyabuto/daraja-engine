"""Check the vendored contract and accidental credential file tracking locally/CI."""

import hashlib
import pathlib
import subprocess
import tomllib

root = pathlib.Path(__file__).resolve().parents[1]
proto = root / "crates/cdk-daraja-processor/proto/payment_processor.proto"
assert hashlib.sha256(proto.read_bytes()).hexdigest() == (
    "1b618a8f0bb0ecfc704e3b856697b524843bb944ae6bcde848698c06cd7b37ee"
)
paths = subprocess.check_output(
    ["git", "ls-files", "-z"], cwd=root
).decode().split("\0")
for name in filter(None, paths):
    path = pathlib.PurePosixPath(name)
    assert not any(part in {"secrets", "credentials", "target"} for part in path.parts), name
    assert path.suffix not in {".pem", ".key", ".p12", ".pfx", ".db", ".sqlite"}, name
    assert not (path.name.startswith(".env") and path.name != ".env.example"), name
    assert path.name not in {"config.toml", "config.local.toml"}, name
config = tomllib.loads((root / "config.example.toml").read_text())
assert all(value == "REPLACE_LOCALLY" for section in ["collection", "disbursement"]
           for value in config[section].values())
assert not any(config["capabilities"].values())
print("Pinned proto checksum, tracked-file policy and configuration placeholders: OK")
