#!/usr/bin/env python3
"""Render cargo-about's JSON into THIRD-PARTY-NOTICES.md.

Reads cargo-about's JSON on stdin and takes a `cargo metadata` dump as its one argument; writes
markdown on stdout. Driven by scripts/gen-third-party-notices.sh, which is the entry point.

cargo-about groups crates by the *bytes* of the licence file they ship, so one SPDX identifier comes
back as dozens of entries: crates under the same licence ship it wrapped at different widths, titled
differently, with or without its appendix, and each with its own copyright line. Reproducing every
copy in full costs a third of a megabyte to state the same terms over and over.

What these licences condition redistribution on is the *notice*, so this lists every crate with the
copyright line its own licence file states, and reproduces the terms once. Copies are collapsed only
where `terms()` says the difference is not a term; a copy whose terms genuinely differ keeps its own
block. Nothing is dropped, nothing is reworded: a crate whose file carries no copyright line is still
listed, and each reproduced text is one crate's file verbatim rather than a reconstruction.
"""

import json
import os
import re
import sys

# A notice line, as opposed to the many sentences inside a licence body that merely use the word:
# it opens the line, capitalised or as the symbol, and names a year or a (c). The unfilled Apache
# appendix placeholder is boilerplate asking to be replaced, not a notice about anyone.
COPYRIGHT = re.compile(r"^(Copyright|COPYRIGHT|©)\b")
IDENTIFIES = re.compile(r"\b(19|20)\d{2}\b|\(c\)|©", re.IGNORECASE)
PLACEHOLDER = re.compile(r"\[yyyy\]|\[name of copyright owner\]|<YEAR>|<COPYRIGHT HOLDER>", re.IGNORECASE)


def notices(text):
    seen = []
    for line in text.splitlines():
        line = line.strip()
        if not COPYRIGHT.match(line) or not IDENTIFIES.search(line):
            continue
        if PLACEHOLDER.search(line) or line in seen:
            continue
        seen.append(line)
    return seen


def plural(n):
    return f"{n} crate" if n == 1 else f"{n} crates"


def terms(text):
    """A licence copy reduced to the words that ARE the terms — what two copies must share for one of
    them to stand in for both.

    Crates ship the same licence typeset differently: wrapped at another width, with `http` rather
    than `https`, with the clause markers of Apache §4 dropped, with or without the appendix that
    tells an author how to apply the licence. None of that is a term, and treating it as one is what
    turns 137 Apache-2.0 crates into 25 reproductions of the same page.
    """
    # Everything after this line is the appendix and any notice the crate attached — not terms.
    cut = text.upper().find("END OF TERMS AND CONDITIONS")
    body = text[:cut] if cut != -1 else text

    kept = []
    heading = True  # Titles differ ("MIT License" / "The MIT License (MIT)" / none at all).
    for line in body.splitlines():
        line = line.strip()
        if not line or COPYRIGHT.match(line) or PLACEHOLDER.search(line):
            continue
        if heading and len(line.split()) <= 8:
            continue
        heading = False
        kept.append(line)

    words = " ".join(kept).lower()
    words = re.sub(r"\([a-z]\)", " ", words)  # clause markers: (a), (b), (i)
    words = re.sub(r"https?://", " ", words)
    words = re.sub(r"[^a-z0-9]+", " ", words)
    return " ".join(words.split())


def notice_files(metadata_path, shipped):
    """The upstream NOTICE files of the shipped crates, as (crate, text) pairs.

    Apache-2.0 §4(d) is a condition separate from §4(a): a NOTICE file in the work must be
    reproduced by whoever redistributes it. cargo-about collects LICENSE files and nothing else, so
    these are read from the crate sources the lock file resolves to. Reproduced whole rather than
    filtered — §4(d) does permit dropping notices that pertain to no part of what is distributed,
    but which of them those are is a judgement, not something to decide by pattern.
    """
    metadata = json.load(open(metadata_path))
    found = []
    for pkg in metadata["packages"]:
        key = (pkg["name"], pkg["version"])
        if key not in shipped:
            continue
        root = os.path.dirname(pkg["manifest_path"])
        for entry in sorted(os.listdir(root)):
            if entry.upper().startswith("NOTICE") and os.path.isfile(os.path.join(root, entry)):
                with open(os.path.join(root, entry), encoding="utf-8") as fh:
                    found.append((f"{pkg['name']} {pkg['version']}", entry, fh.read().strip()))
    return sorted(found)


def main():
    about = json.load(sys.stdin)
    # overview is ordered by crate count, which is the order the sections should read in.
    order = [entry["id"] for entry in about["overview"]]
    counts = {entry["id"]: entry["count"] for entry in about["overview"]}
    names = {entry["id"]: entry["name"] for entry in about["overview"]}

    by_id = {}
    for lic in about["licenses"]:
        by_id.setdefault(lic["id"], []).append(lic)

    out = ["## Summary", ""]
    for spdx in order:
        out.append(f"- [`{spdx}`](#{spdx.lower()}) — {plural(counts[spdx])}")

    for spdx in order:
        out += ["", f"## {names[spdx]} (`{spdx}`)", ""]

        # Group the variants by their terms rather than by their bytes. Collapsing them is only sound
        # where the difference IS the notice; a variant whose terms genuinely differ gets its own
        # block, so nothing is asserted to say something it does not.
        groups = {}
        for lic in by_id[spdx]:
            groups.setdefault(terms(lic["text"]), []).append(lic)
        ranked = sorted(groups.values(), key=lambda g: sum(len(l["used_by"]) for l in g), reverse=True)

        if len(ranked) > 1:
            out.append(f"{plural(counts[spdx])}, under {len(ranked)} variants of these terms.")
            out.append("")

        for variants in ranked:
            crates = {}
            for lic in variants:
                for use in lic["used_by"]:
                    pkg = use["crate"]
                    crates[(pkg["name"], pkg["version"])] = notices(lic["text"])

            verb = "is" if len(crates) == 1 else "are"
            out.append(f"{plural(len(crates))} {verb} distributed under the terms below:")
            out.append("")
            for (name, version), lines in sorted(crates.items()):
                out.append(f"- **{name} {version}**" + (f" — {'; '.join(lines)}" if lines else ""))

            example = variants[0]["used_by"][0]["crate"]
            out += [
                "",
                f"Terms, reproduced from the copy shipped by `{example['name']} {example['version']}`:",
                "",
                "```",
                variants[0]["text"].strip(),
                "```",
                "",
            ]
        out.pop()

    shipped = {
        (use["crate"]["name"], use["crate"]["version"])
        for lic in about["licenses"]
        for use in lic["used_by"]
    }
    found = notice_files(sys.argv[1], shipped)
    if found:
        out += [
            "",
            "# NOTICE files",
            "",
            "Apache-2.0 §4(d) requires a redistributor to reproduce the attribution notices of any",
            "`NOTICE` file the work ships. These are those files, verbatim.",
        ]
    for crate, filename, text in found:
        out += ["", f"## {filename} shipped by {crate}", "", "```", text, "```"]

    sys.stdout.write("\n".join(out) + "\n")


if __name__ == "__main__":
    main()
