import socket
import subprocess
import time
from pathlib import Path

import httpx
import pytest

from caldav_mcp.accounts import AccountConfig, Settings


@pytest.fixture(scope="session")
def radicale(tmp_path_factory):
    root = tmp_path_factory.mktemp("radicale")
    users = root / "users"
    users.write_text("alice:alice-pass\nbob:bob-pass\n")
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    config = root / "config"
    config.write_text(f"""[server]
hosts = 127.0.0.1:{port}
[auth]
type = htpasswd
htpasswd_filename = {users}
htpasswd_encryption = plain
delay = 0
[rights]
type = owner_only
[storage]
filesystem_folder = {root / "collections"}
[sharing]
type = files
collection_by_map = True
permit_create_map = True
[logging]
level = warning
""")
    with (root / "server.log").open("w+") as log:
        process = subprocess.Popen(
            [str(Path(".venv/bin/python").absolute()), "-m", "radicale", "--config", str(config)],
            stdout=log,
            stderr=log,
        )
        base = f"http://127.0.0.1:{port}/"
        try:
            for _ in range(100):
                if process.poll() is not None:
                    log.seek(0)
                    pytest.fail(log.read())
                try:
                    httpx.get(base, timeout=0.2)
                    break
                except httpx.TransportError:
                    time.sleep(0.05)
            else:
                pytest.fail("Radicale did not start")
            yield base
        finally:
            process.terminate()
            process.wait(timeout=10)


@pytest.fixture
def settings(radicale, monkeypatch):
    monkeypatch.setenv("ALICE_PASSWORD", "alice-pass")
    monkeypatch.setenv("BOB_PASSWORD", "bob-pass")
    return Settings(
        accounts=tuple(
            AccountConfig(
                id=user,
                url=radicale,
                username=user,
                password_env=f"{user.upper()}_PASSWORD",
                sharing_url=radicale + ".sharing/v1/",
            )
            for user in ("alice", "bob")
        )
    )
