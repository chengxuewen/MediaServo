#!/usr/bin/env python3
"""Shared helpers for the agent single-source generator (H5 scope).

Standard-library only. Exit-code contract shared by every generator/check:
  0 = clean, 1 = findings (check) / actions applied (sync), 2 = guard violation.
"""

import json
import os

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

EXIT_OK = 0
EXIT_FINDINGS = 1
EXIT_GUARD = 2


def load_json(path):
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def json_text(obj):
    return json.dumps(obj, indent=2, ensure_ascii=False) + "\n"


def rel(path, start):
    return os.path.relpath(path, start).replace(os.sep, "/")


def guard_target(path, source_real):
    """Return a reason string when writing path would touch the canonical tree."""
    target_real = os.path.realpath(path)
    if target_real == source_real or target_real.startswith(source_real + os.sep):
        return "target resolves inside the canonical source: %s" % path
    return None


def region_bounds(text, begin, end):
    if begin not in text or end not in text:
        return None
    head, rest = text.split(begin, 1)
    body, tail = rest.split(end, 1)
    return head, body, tail


def replace_region(text, begin, end, block):
    bounds = region_bounds(text, begin, end)
    if bounds is None:
        return None
    head, _body, tail = bounds
    return head + begin + "\n" + block.rstrip("\n") + "\n" + end + tail


def place_region(text, begin, end, block, before=None):
    """Replace the marked region, or insert it (optionally before another marker)."""
    if begin in text and end in text:
        return replace_region(text, begin, end, block)
    section = begin + "\n" + block.rstrip("\n") + "\n" + end + "\n"
    if before and before in text:
        head, tail = text.split(before, 1)
        return head + section + "\n" + before + tail
    return text.rstrip("\n") + "\n\n" + section


def read_text(path):
    with open(path, encoding="utf-8") as handle:
        return handle.read()


def write_text(path, text):
    os.makedirs(os.path.dirname(os.path.abspath(path)), exist_ok=True)
    with open(path, "w", encoding="utf-8") as handle:
        handle.write(text)


def skill_description(path):
    """First `description:` value from a SKILL.md frontmatter block."""
    try:
        text = read_text(path)
    except OSError:
        return ""
    lines = text.splitlines()
    if not lines or lines[0].strip() != "---":
        return ""
    for line in lines[1:]:
        if line.strip() == "---":
            break
        if line.startswith("description:"):
            value = line.split(":", 1)[1].strip()
            return value.strip('"').strip("'")
    return ""
