import contextlib
import importlib.util
import io
from pathlib import Path
import struct
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    "package_linux", Path(__file__).resolve().parents[2] / "scripts/package-linux.py"
)
packages = importlib.util.module_from_spec(spec)
spec.loader.exec_module(packages)

REVISION = "1" * 40
VERSION = f"0.1.0-dev.g{REVISION}"


class PackageTests(unittest.TestCase):
    def test_tag_requires_matching_versions_and_checksums(self):
        with tempfile.TemporaryDirectory() as path:
            root = Path(path)
            arm = "rustward-linux-aarch64"
            binaries = root / "binaries"
            binaries.mkdir()

            def bundle(architecture, version):
                for name in packages.EXECUTABLES:
                    header = b"\x7fELF\x02\x01\x01" + bytes(11) + struct.pack("<H", packages.MACHINES[architecture])
                    (binaries / name).write_bytes(header)
                (binaries / "rustward-version").write_text(version)
                with contextlib.redirect_stdout(io.StringIO()):
                    packages.package(binaries, root / f"rustward-linux-{architecture}",
                                     architecture, REVISION, "0.1.0")

            bundle("x86_64", VERSION)
            bundle("aarch64", VERSION)
            self.assertEqual(packages.tag_for_downloads(root, REVISION), f"v{VERSION}")
            with self.assertRaisesRegex(ValueError, "source commit"):
                packages.tag_for_version(VERSION, "2" * 40)
            bundle("aarch64", f"0.2.0-dev.g{REVISION}")
            with self.assertRaisesRegex(ValueError, "different versions"):
                packages.tag_for_downloads(root, REVISION)
            (root / arm / f"{arm}.tar.gz.sha256").write_text("incorrect")
            with self.assertRaisesRegex(ValueError, "Checksum differs"):
                packages.tag_for_downloads(root, REVISION)
