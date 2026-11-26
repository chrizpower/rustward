"""Run local and SSH acceptance inside the disposable image."""
import json
from pathlib import Path
import traceback

from inside import run
from nspawn_ssh import run as run_ssh


def main():
    root = Path("/opt/rustward-test")
    revision = (root / "source-revision").read_text().strip()
    result = {"status": "failed", "source_revision": revision}
    ssh_evidence = Path("/opt/rustward-ssh-evidence")
    try:
        run(root / "fixture", root / "tools", "rustward", revision)
        Path("/opt/rustward-acceptance-tools").symlink_to(root / "tools", target_is_directory=True)
        run_ssh(ssh_evidence, revision)
        result["status"] = "passed"
    except Exception as error:
        result["reason"] = str(error)
        traceback.print_exc()
    finally:
        if (ssh_evidence / "credential-scan.json").is_file():
            scan = json.loads((ssh_evidence / "credential-scan.json").read_text())
            if scan.get("status") == "passed":
                ssh_evidence.rename(root / "ssh")
        (root / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
