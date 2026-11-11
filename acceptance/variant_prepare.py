"""Publish independent A/B catalogs for A08; never remove the shared runtime."""
import json
from pathlib import Path
import pwd
import shutil
from prepare import create_root, configuration, write, identify
from manifest import tree_manifest, source


def prepare(root, original, tools, revision):
    metadata = json.loads((original / "fixture.json").read_text())
    if metadata["source_revision"] != revision: raise ValueError("source revision changed")
    create_root(root, pwd.getpwnam("rustward"))
    shared = original / "runtimes" / metadata["runtime_id"]
    host_id = metadata["host_id"]
    shutil.copyfile(original / "hosts" / f"{host_id}.toml", root / "hosts" / f"{host_id}.toml")
    variants = {}
    for label in ("A", "B"):
        stage = root / ("pending-" + label)
        shutil.copytree(shared, stage)
        (stage / "runtime.toml").unlink()
        write(stage / "selection.txt", label + "\n")
        directories, files = tree_manifest(stage)
        write(stage / "runtime.toml", source({"version":1,"host":host_id,"directories":directories,"files":files}))
        identity = identify(tools / "acceptance", "runtime", stage / "runtime.toml")
        stage.rename(root / "runtimes" / identity)
        variants[label] = identity
        directory = "configuration" if label == "A" else "configuration-b"
        configuration(root, root / "runtimes" / identity, remote=True, directory=directory)
        tasks = root / directory / "tasks"
        mode = "a" if label == "A" else "b"
        shutil.copyfile(tasks / f"remote-runtime-{mode}.toml", tasks / "runtime-selection.toml")
    metadata.pop("catalog_root", None)
    metadata.update(runtime_id=variants["A"], runtime_b_id=variants["B"])
    write(root / "fixture.json", json.dumps(metadata, indent=2) + "\n")
    return metadata
