import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
LAUNCHER = ROOT / "hermes" / "launch.py"


def load_launcher():
    spec = importlib.util.spec_from_file_location("caldav_plugin", LAUNCHER)
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class HermesPluginTests(unittest.TestCase):
    def test_portable_package_launches_with_profile_config_and_credentials(self):
        manifest = json.loads((ROOT / "plugin.json").read_text())
        self.assertEqual(manifest["$schema"],
                         "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json")
        self.assertEqual(manifest["name"], "caldav-mcp")
        mcp = json.loads((ROOT / "mcp.json").read_text())
        self.assertEqual(mcp["$schema"],
                         "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json")
        server = mcp["mcpServers"]["caldav"]
        self.assertEqual(server["type"], "stdio")
        with tempfile.TemporaryDirectory(prefix="caldav plugin ") as directory:
            data = Path(directory)
            config = data / "config.toml"
            config.write_text('[[accounts]]\npassword_env = "TEAM_PASSWORD"\n')
            binary = data / "test-server"
            binary.write_text(
                f"#!{sys.executable}\n"
                "import json, os, sys\n"
                "print(json.dumps({'args': sys.argv[1:], "
                "'password': os.environ['TEAM_PASSWORD'], "
                "'input': sys.stdin.readline().strip()}))\n"
            )
            binary.chmod(0o700)
            user_env = {
                "CALDAV_MCP_CONFIG": str(config),
                "CALDAV_MCP_BINARY": str(binary),
                "CALDAV_MCP_CREDENTIALS": json.dumps({"TEAM_PASSWORD": "test-secret"}),
            }
            env = {"PATH": os.environ["PATH"], "PLUGIN_ROOT": str(ROOT),
                   "PLUGIN_DATA": str(data)}
            for key, value in server["env"].items():
                self.assertEqual(value, "${" + key + "}")
                if key in user_env:
                    env[key] = user_env[key]
            args = [arg.replace("${PLUGIN_ROOT}", str(ROOT)) for arg in server["args"]]
            result = subprocess.run([server["command"], *args], env=env,
                                    input="mcp request\n", text=True, capture_output=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(json.loads(result.stdout), {
                "args": ["--config", str(config)],
                "password": "test-secret", "input": "mcp request",
            })

    def test_missing_config_fails_before_downloading_without_stdout(self):
        module = load_launcher()
        with tempfile.TemporaryDirectory() as directory:
            env = {"PLUGIN_DATA": directory}
            with patch.dict(os.environ, env, clear=True), patch.object(module, "urlopen") as download:
                with self.assertRaisesRegex(ValueError, "CALDAV_MCP_CONFIG"):
                    module.launch()
                download.assert_not_called()

    def test_credentials_reject_invalid_values_without_disclosing_secrets(self):
        module = load_launcher()
        for value in ('[]', '{"TOKEN": 42}', '{"BAD=NAME":"secret"}',
                      '{"TOKEN":"secret\\u0000"}', 'secret-invalid-json'):
            with self.subTest(value=value):
                with self.assertRaises(ValueError) as caught:
                    module.Credentials.parse(value)
                self.assertNotIn("secret", str(caught.exception))

    def test_release_is_verified_cached_and_only_binary_is_extracted(self):
        module = load_launcher()
        target = "x86_64-unknown-linux-musl"
        package = f"caldav-mcp-{target}"
        archive = io.BytesIO()
        with tarfile.open(fileobj=archive, mode="w:gz") as tar:
            for name, content in ((f"{package}/caldav-mcp", b"binary"),
                                  ("../../escaped", b"unwanted")):
                info = tarfile.TarInfo(name)
                info.size = len(content)
                tar.addfile(info, io.BytesIO(content))
        payload = archive.getvalue()
        checksum = f"{hashlib.sha256(payload).hexdigest()}  ./{package}.tar.gz\n".encode()
        with tempfile.TemporaryDirectory() as directory:
            release = module.Release("0.1.0", target)
            with patch.object(module, "urlopen", side_effect=[io.BytesIO(checksum), io.BytesIO(payload)]) as download:
                binary = release.install(Path(directory))
                self.assertEqual(binary.read_bytes(), b"binary")
                self.assertTrue(os.access(binary, os.X_OK))
                self.assertEqual(release.install(Path(directory)), binary)
                self.assertEqual(download.call_count, 2)
                for call in download.call_args_list:
                    self.assertTrue(call.args[0].startswith(
                        "https://github.com/ROCKTAKEY/caldav-mcp/releases/download/v0.1.0/"))
            self.assertEqual([p.name for p in Path(directory).rglob("*") if p.is_file()],
                             ["caldav-mcp"])

    def test_bad_checksum_never_installs_binary(self):
        module = load_launcher()
        target = "aarch64-unknown-linux-musl"
        checksum = f"{'0' * 64}  caldav-mcp-{target}.tar.gz\n".encode()
        with tempfile.TemporaryDirectory() as directory:
            with patch.object(module, "urlopen", side_effect=[io.BytesIO(checksum), io.BytesIO(b"corrupt")]):
                with self.assertRaisesRegex(ValueError, "checksum"):
                    module.Release("0.1.0", target).install(Path(directory))
            self.assertFalse(list(Path(directory).rglob("caldav-mcp")))


if __name__ == "__main__":
    unittest.main()
