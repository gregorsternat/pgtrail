#!/usr/bin/env python3
"""Check repository-local knowledge links, ownership, and review dates (stdlib only)."""
import datetime as dt
from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parents[1]
ENTRYPOINTS = {"AGENTS.md", "README.md", "CONTRIBUTING.md", "docs/index.md"}
METADATA = re.compile(r"<!-- owner: ([^;]+); reviewed: (\d{4}-\d{2}-\d{2}) -->")
LINK = re.compile(r"!?\[[^\]\n]*\]\(<?([^\s)>]+)>?(?:\s+\"[^\"]*\")?\)")
FENCE = re.compile(r"^\s*(`{3,}|~{3,})")


def prose(text):
    """Exclude fenced examples; knowledge links use inline Markdown syntax."""
    fence = None
    lines = []
    for line in text.splitlines():
        match = FENCE.match(line)
        if match:
            marker = match[1]
            if fence is None:
                fence = marker
            elif marker[0] == fence[0] and len(marker) >= len(fence):
                fence = None
            continue
        if fence is None:
            lines.append(line)
    return "\n".join(lines)


def anchors(text):
    counts = {}
    result = set()
    for line in prose(text).splitlines():
        match = re.match(r"^#{1,6}\s+(.+?)(?:\s+#+)?$", line)
        if match:
            title = match[1].lower().replace("`", "")
            slug = re.sub(r"[^\w\- ]", "", title).replace(" ", "-")
            count = counts.get(slug, 0)
            result.add(f"{slug}-{count}" if count else slug)
            counts[slug] = count + 1
    return result


def check(root, files, today):
    errors = []
    documents = {name for name in files if name.endswith(".md") and (name in ENTRYPOINTS or name.startswith("docs/"))}
    for name in sorted(ENTRYPOINTS - documents):
        errors.append(f"{name}: required entry point missing; restore the repository map")
    graph = {name: set() for name in documents}
    for name in sorted(documents):
        content = (root / name).read_text()
        metadata = METADATA.search(content)
        if not metadata:
            errors.append(f"{name}: add <!-- owner: maintainers; reviewed: YYYY-MM-DD --> after reviewing the content")
        else:
            try:
                reviewed = dt.date.fromisoformat(metadata[2])
                age = (today - reviewed).days
                # Completed plans preserve historical evidence, not a current claim.
                if age < 0 or (age > 90 and not name.startswith("docs/exec-plans/completed/")):
                    errors.append(f"{name}: review date {reviewed} is future or older than 90 days; verify linked code and update facts before renewing it")
            except ValueError:
                errors.append(f"{name}: invalid review date {metadata[2]}")
        for match in LINK.finditer(prose(content)):
            target = urlsplit(match[1])
            if target.scheme or target.netloc:
                continue
            destination = ((root / name).parent / unquote(target.path)).resolve() if target.path else root / name
            try:
                relative = destination.relative_to(root.resolve()).as_posix()
            except ValueError:
                errors.append(f"{name}: link escapes repository: {match[1]}; use a repository-relative path")
                continue
            if relative not in files:
                errors.append(f"{name}: missing or ignored target {match[1]}; fix the link or version its target")
                continue
            if relative in graph:
                graph[name].add(relative)
            if target.fragment and relative.endswith(".md") and unquote(target.fragment) not in anchors(destination.read_text()):
                errors.append(f"{name}: missing heading {match[1]}; update the fragment")
    reached = set()
    pending = [name for name in ENTRYPOINTS if name in graph]
    while pending:
        name = pending.pop()
        if name not in reached:
            reached.add(name)
            pending.extend(graph[name] - reached)
    for name in sorted(documents - reached):
        errors.append(f"{name}: orphan document; link it from docs/index.md or a reachable guide")
    agents = root / "AGENTS.md"
    if agents.exists() and len(agents.read_text().splitlines()) > 100:
        errors.append("AGENTS.md: exceeds 100 lines; move detail to its owning guide and keep pointers here")
    return errors


def main():
    tracked = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=ROOT).decode().split("\0")
    files = {name for name in tracked if name and (ROOT / name).is_file()}
    errors = check(ROOT, files, dt.datetime.now(dt.timezone.utc).date())
    if errors:
        print("Documentation checks failed:\n" + "\n".join(f"- {error}" for error in errors), file=sys.stderr)
        return 1
    print("Documentation checks passed: links, headings, reachability, owners, review dates, and AGENTS.md size")
    return 0


if __name__ == "__main__":
    sys.exit(main())
