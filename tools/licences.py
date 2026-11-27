#!/usr/bin/env python3
"""Writes the licence notices that go out with the app.

    python tools/licences.py            (from the top of the repository)

The result, android/app/src/main/assets/licences.txt, is shown in the app under
Settings, Licences, and is published beside the download on the website. It holds this
project's own licence, then every library built into the app with its licence text.
Run it again whenever a dependency changes, before a release build.

Rust libraries are read from `cargo metadata`, and their licence files from the copies
cargo downloaded. Android libraries are read from Gradle; they must all come from the
publishers listed in ANDROID_PUBLISHERS, whose licences are known. Anything else stops
the script so that someone looks at it.
"""
import hashlib
import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "android", "app", "src", "main", "assets", "licences.txt")
SITE = os.path.join(ROOT, "site", "licences.txt")
TARGET = "aarch64-linux-android"
# Publishers of the Android libraries, and the licence each uses. (JNA offers Apache 2.0 or
# LGPL 2.1; this project takes it under Apache 2.0.)
ANDROID_PUBLISHERS = {
    "androidx.": "Apache-2.0", "org.jetbrains": "Apache-2.0", "com.google.zxing": "Apache-2.0", "net.java.dev.jna": "Apache-2.0",
    "com.google.guava": "Apache-2.0", "org.jspecify": "Apache-2.0", "com.google.errorprone": "Apache-2.0",
    "com.google.auto.value": "Apache-2.0", "com.google.code.findbugs": "Apache-2.0", "com.google.dagger": "Apache-2.0",
    "com.google.j2objc": "Apache-2.0", "jakarta.inject": "Apache-2.0", "javax.inject": "Apache-2.0",
    "org.checkerframework": "MIT", "net.zetetic": "BSD-3-Clause",
}
CHECKER_MIT = """Checker Framework qualifiers
Copyright 2004-present by the Checker Framework developers

MIT License:

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in
all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
THE SOFTWARE."""
# What a library may declare without anyone needing to look: licences that allow use in a
# program under other terms. A copyleft-only library must never slip in unnoticed.
ALLOWED = re.compile(r"\b(MIT|Apache-2\.0|BSD-[123]-Clause|ISC|Zlib|Unicode-3\.0|MPL-2\.0|Unlicense|CDLA-Permissive-2\.0|BSL-1\.0|0BSD|CC0-1\.0)\b")
LICENCE_FILE = re.compile(r"^(LICEN[SC]E|COPYING|NOTICE|UNLICEN[SC]E|COPYRIGHT)", re.I)


def run(command, cwd):
    return subprocess.run(command, cwd=cwd, check=True, capture_output=True, text=True, encoding="utf-8", shell=isinstance(command, str)).stdout


def rust_libraries():
    """(name, version, declared licence, where it comes from, [licence texts]) for every crate built into the app."""
    meta = json.loads(run(["cargo", "metadata", "--format-version", "1", "--filter-platform", TARGET], ROOT))
    packages = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    start = next(i for i, p in packages.items() if p["name"] == "deaddrop-ffi")
    seen, queue = {start}, [start]
    while queue:
        for dep in nodes[queue.pop()]["deps"]:
            built_in = any(kind["kind"] is None for kind in dep["dep_kinds"])  # not build-time or test-only
            if built_in and dep["pkg"] not in seen:
                seen.add(dep["pkg"])
                queue.append(dep["pkg"])
    found = []
    for package_id in seen:
        package = packages[package_id]
        if package["source"] is None:  # part of this project
            continue
        declared = package.get("license") or ""
        if not ALLOWED.search(declared):
            sys.exit(f"{package['name']} {package['version']} declares '{declared}': check it before it ships")
        folder = os.path.dirname(package["manifest_path"])
        texts = []
        for name in sorted(os.listdir(folder)):
            path = os.path.join(folder, name)
            if LICENCE_FILE.match(name) and os.path.isfile(path):
                texts.append(open(path, encoding="utf-8", errors="replace").read().replace("\r\n", "\n").strip())
        found.append((package["name"], package["version"], declared, package.get("repository") or "crates.io", texts))
    return sorted(found)


def android_libraries():
    """Every library Gradle puts in the release app, as (group:name:version, licence)."""
    # `sh` so that this also works from Git Bash on Windows, where the repository has no gradlew.bat.
    report = run(["sh", "./gradlew", "-q", ":app:dependencies", "--configuration", "releaseRuntimeClasspath"], os.path.join(ROOT, "android"))
    found = set()
    for group, name, asked, resolved in re.findall(r"--- ([\w.\-]+):([\w.\-]+):?([\w.\-]*)(?: -> ([\w.\-]+))?", report):
        version = resolved or asked
        if not version:
            continue
        licence = next((licence for publisher, licence in ANDROID_PUBLISHERS.items() if group.startswith(publisher)), None)
        if licence is None:
            sys.exit(f"{group}:{name} is from a publisher whose licence is not recorded here: check it")
        found.add((f"{group}:{name}:{version}", licence))
    if not found:
        sys.exit("Gradle listed no libraries")
    return sorted(found)


def main():
    rust = rust_libraries()
    android = android_libraries()
    own = open(os.path.join(ROOT, "LICENSE.md"), encoding="utf-8").read().strip()
    notice = next(line for line in open(os.path.join(ROOT, "README.md"), encoding="utf-8") if line.startswith("Required Notice:")).strip()

    out = ["DECENTRALIZED DEAD DROP", "", notice, "", "Licensed under the PolyForm Noncommercial License 1.0.0:", "", own, "", "", "=" * 78,
           "SOFTWARE THIS APP IS BUILT ON", "=" * 78, "",
           "Each library below keeps its own licence. The licence texts follow the two lists.", "",
           "Rust libraries (name, version, licence, source):", ""]
    out += [f"  {name} {version}  [{declared}]  {source}" for name, version, declared, source, _ in rust]
    out += ["", "Android and Kotlin libraries (name, version, licence):", ""]
    out += [f"  {library}  [{licence}]" for library, licence in android]

    by_text = {}
    without = []
    for name, version, declared, _, texts in rust:
        if not texts:
            without.append(f"{name} {version} [{declared}]")
        for text in texts:
            key = hashlib.sha256(" ".join(text.split()).encode()).hexdigest()
            by_text.setdefault(key, (text, []))[1].append(f"{name} {version}")
    if not any("Apache License" in text and "Version 2.0" in text for text, _ in by_text.values()):
        sys.exit("no copy of the Apache License 2.0 was found to include")
    if without:
        out += ["", "These Rust libraries ship no licence file of their own; their declared licences are given above:", ""]
        out += [f"  {entry}" for entry in without]

    if any(library.startswith("org.checkerframework") for library, _ in android):
        by_text["checker"] = (CHECKER_MIT, ["org.checkerframework:checker-qual"])
    if any(library.startswith("net.zetetic") for library, _ in android):
        # Its licence, kept beside this script, with a note of what it is built on.
        zetetic = open(os.path.join(ROOT, "tools", "licences", "sqlcipher-android.txt"), encoding="utf-8").read().strip()
        by_text["sqlcipher"] = (zetetic, ["net.zetetic:sqlcipher-android"])
    apache = [library for library, licence in android if licence == "Apache-2.0"]
    for text, users in by_text.values():
        if text.lstrip().startswith("Apache License") and "Version 2.0" in text[:200]:
            users.append(f"and the {len(apache)} Android and Kotlin libraries marked [Apache-2.0] above")
            break

    out += ["", "", "=" * 78, "LICENCE TEXTS", "=" * 78]
    for text, users in sorted(by_text.values(), key=lambda item: (-len(item[1]), item[1])):
        out += ["", "-" * 78, "Applies to: " + ", ".join(users), "-" * 78, "", text]

    result = "\n".join(out) + "\n"
    for path in (OUT, SITE):
        os.makedirs(os.path.dirname(path), exist_ok=True)
        open(path, "w", encoding="utf-8", newline="\n").write(result)
    print(f"{len(rust)} Rust libraries, {len(android)} Android libraries, {len(by_text)} licence texts, {len(result) // 1024} KB")


if __name__ == "__main__":
    main()
