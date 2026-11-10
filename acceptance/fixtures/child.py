import os
from pathlib import Path
import sys
import time

sys.path.insert(0, str(Path(__file__).parent))
from common import process, save

save(Path(os.environ["RUSTWARD_WORK_DIRECTORY"]) / "child.json", process())
while True:
    time.sleep(0.1)
