"""Run the existing local acceptance suite inside the disposable image."""
import json
from pathlib import Path
import traceback

from inside import run


def main():
    root = Path("/opt/rustward-test")
    revision = (root / "source-revision").read_text().strip()
    result = {"status": "failed", "source_revision": revision}
    try:
        run(root / "fixture", root / "tools", "rustward", revision)
        result["status"] = "passed"
    except Exception as error:
        result["reason"] = str(error)
        traceback.print_exc()
    finally:
        (root / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
