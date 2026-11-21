"""Require TypeScript for console and browser test source."""
from html.parser import HTMLParser
import os
from pathlib import Path

root = Path(__file__).resolve().parents[2]
generated = {"node_modules", "dist", "test-results", "playwright-report"}
javascript = {".js", ".jsx", ".cjs", ".mjs"}


class Scripts(HTMLParser):
    def __init__(self, svelte):
        super().__init__()
        self.svelte = svelte
        self.in_html_script = False
        self.javascript = False

    def handle_starttag(self, tag, attrs):
        if not self.svelte and any(name.startswith("on") for name, _ in attrs):
            self.javascript = True
        if tag != "script":
            return
        values = dict(attrs)
        if self.svelte:
            self.javascript |= values.get("lang") != "ts"
        else:
            self.in_html_script = True
            self.javascript |= Path(values.get("src") or "").suffix != ".ts"

    def handle_endtag(self, tag):
        if tag == "script":
            self.in_html_script = False

    def handle_data(self, data):
        if self.in_html_script and data.strip():
            self.javascript = True


def handwritten_javascript(path):
    suffix = path.suffix.lower()
    if suffix in javascript:
        return True
    if suffix in {".html", ".svelte"}:
        parser = Scripts(svelte=suffix == ".svelte")
        parser.feed(path.read_text(encoding="utf-8"))
        parser.close()
        return parser.javascript
    return False


violations = []
for source in (root / "web", root / "tests/web"):
    for parent, directories, files in os.walk(source):
        if Path(parent) == source:
            directories[:] = [name for name in directories if name not in generated]
        for name in files:
            path = Path(parent) / name
            if handwritten_javascript(path):
                violations.append(path.relative_to(root))

if violations:
    for path in sorted(violations):
        print(f"{path}: Use TypeScript for console and browser test source.")
    raise SystemExit(1)
