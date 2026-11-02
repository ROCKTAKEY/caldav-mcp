# /// script
# requires-python = ">=3.11"
# dependencies = ["certifi"]
# ///
"""Launch the release binary using the client-managed plugin data directory."""

import hashlib
import io
import json
import os
import platform
import re
import ssl
import sys
import tarfile
import tempfile
import tomllib
from collections.abc import Mapping
from dataclasses import dataclass
from pathlib import Path
from types import MappingProxyType
from urllib.request import urlopen

import certifi


@dataclass(frozen=True)
class Credentials:
    values: Mapping[str, str]

    @classmethod
    def parse(cls, source: str) -> "Credentials":
        message = "credentials.json must be a JSON object mapping environment names to strings"
        try:
            values = json.loads(source)
        except ValueError:
            raise ValueError(message) from None
        if not isinstance(values, dict) or any(
            re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", key) is None
            or not isinstance(value, str)
            or "\0" in value
            for key, value in values.items()
        ):
            raise ValueError(message)
        return cls(MappingProxyType(values))


@dataclass(frozen=True)
class Release:
    version: str
    target: str

    @classmethod
    def current(cls) -> "Release":
        machines = {"x86_64": "x86_64", "aarch64": "aarch64", "arm64": "aarch64"}
        machine = machines.get(platform.machine())
        if platform.system() != "Linux" or machine is None:
            raise ValueError(
                "Automatic installation supports Linux x86_64/ARM64; set binary in launcher.toml to your own build"
            )
        cargo = Path(__file__).resolve().parents[1] / "Cargo.toml"
        version = tomllib.loads(cargo.read_text())["package"]["version"]
        if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?", version):
            raise ValueError("Cargo.toml contains an unsupported release version")
        return cls(version, f"{machine}-unknown-linux-musl")

    def install(self, data: Path) -> Path:
        destination = data / "bin" / self.version / self.target / "caldav-mcp"
        if destination.is_file() and os.access(destination, os.X_OK):
            return destination
        return self.download().install(destination)

    def download(self) -> "VerifiedBinary":
        package = f"caldav-mcp-{self.target}"
        base = (
            f"https://github.com/ROCKTAKEY/caldav-mcp/releases/download/v{self.version}"
        )
        print(
            f"caldav-mcp: downloading v{self.version} ({self.target})", file=sys.stderr
        )
        context = ssl.create_default_context(cafile=certifi.where())
        with urlopen(f"{base}/SHA256SUMS", timeout=60, context=context) as response:
            checksums = response.read().decode()
        expected = None
        for line in checksums.splitlines():
            fields = line.split()
            if len(fields) == 2 and fields[1].removeprefix("./") == f"{package}.tar.gz":
                expected = fields[0]
        if expected is None:
            raise ValueError("release checksum is missing")
        with urlopen(
            f"{base}/{package}.tar.gz", timeout=60, context=context
        ) as response:
            archive = response.read()
        return VerifiedBinary.from_archive(archive, expected, f"{package}/caldav-mcp")


@dataclass(frozen=True)
class VerifiedBinary:
    content: bytes

    @classmethod
    def from_archive(
        cls, archive: bytes, checksum: str, member_name: str
    ) -> "VerifiedBinary":
        if hashlib.sha256(archive).hexdigest() != checksum:
            raise ValueError("release checksum mismatch")
        with tarfile.open(fileobj=io.BytesIO(archive), mode="r:gz") as tar:
            member = tar.getmember(member_name)
            if not member.isfile():
                raise ValueError("release binary must be a regular file")
            return cls(tar.extractfile(member).read())

    def install(self, destination: Path) -> Path:
        destination.parent.mkdir(parents=True, exist_ok=True)
        # Concurrent plugin sessions must never observe a partially written executable.
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(
                dir=destination.parent, delete=False
            ) as output:
                temporary = Path(output.name)
                output.write(self.content)
                output.flush()
                os.fchmod(output.fileno(), 0o700)
            temporary.replace(destination)
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)
        return destination


def launch() -> None:
    supplied_data = os.environ.get("PLUGIN_DATA", "")
    if not supplied_data or not Path(supplied_data).is_absolute():
        raise ValueError(
            "The plugin client must supply an absolute PLUGIN_DATA directory"
        )
    data = Path(supplied_data)
    config = data / "config.toml"
    if not config.is_file():
        raise ValueError(
            f"Create {config} using examples/config.toml from the plugin package"
        )
    credentials_file = data / "credentials.json"
    credentials = Credentials.parse(
        credentials_file.read_text() if credentials_file.exists() else "{}"
    )
    settings_file = data / "launcher.toml"
    settings = (
        tomllib.loads(settings_file.read_text()) if settings_file.exists() else {}
    )
    override = settings.get("binary")
    if override is not None:
        if not isinstance(override, str) or not override:
            raise ValueError(
                "launcher.toml binary must be an absolute path to an executable"
            )
        binary = Path(override)
        if (
            not binary.is_absolute()
            or not binary.is_file()
            or not os.access(binary, os.X_OK)
        ):
            raise ValueError(
                "launcher.toml binary must be an absolute path to an executable"
            )
    else:
        binary = Release.current().install(data)
    env = dict(os.environ)
    env.update(credentials.values)
    os.execve(binary, [str(binary), "--config", str(config)], env)


if __name__ == "__main__":
    try:
        launch()
    except (OSError, ValueError, KeyError, tarfile.TarError) as error:
        print(f"caldav-mcp plugin: {error}", file=sys.stderr)
        sys.exit(1)
