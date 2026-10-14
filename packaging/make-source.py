#!/usr/bin/env python3
"""Export a committed, vendored source release plus native build recipes.

Never publishes a release. Build packages from the resulting archive, not a
GitHub auto-generated tarball (which lacks dependencies and license notices).
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib


def run(*args, **kwargs):
    return subprocess.check_output(args, text=True, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("--ref", default="HEAD", help="committed git revision")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    commit = run("git", "rev-parse", "--verify", "--end-of-options", args.ref + "^{commit}", cwd=root).strip()
    epoch = run("git", "show", "-s", "--format=%ct", commit, cwd=root).strip()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="epoxy-source-", dir=output) as tmp:
        temp = Path(tmp)
        source = temp / "source"
        source.mkdir()
        with (temp / "git.tar").open("wb") as stream:
            subprocess.run(["git", "archive", commit], cwd=root, stdout=stream, check=True)
        subprocess.run(["tar", "-xf", str(temp / "git.tar"), "-C", str(source)], check=True)
        version = tomllib.loads((source / "epoxy/Cargo.toml").read_text())["package"]["version"]
        # Only plain versions are currently supported by all three package formats.
        if len(version.split(".")) != 3 or not all(part.isdigit() for part in version.split(".")):
            raise SystemExit("Release version must be MAJOR.MINOR.PATCH")
        name = f"epoxy-signing-{version}"
        renamed = temp / name
        source.rename(renamed)
        source = renamed
        config = run("cargo", "vendor", "--locked", "--versioned-dirs", "vendor", cwd=source)
        (source / ".cargo").mkdir(exist_ok=True)
        (source / ".cargo/config.toml").write_text(config)
        metadata = json.loads(run("cargo", "metadata", "--locked", "--offline", "--all-features", "--format-version", "1", cwd=source))
        licenses = source / "packaging/licenses"
        licenses.mkdir()
        inventory = []
        expressions = set()
        for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
            if package["source"] is None:
                continue
            expression = package.get("license")
            if not expression:
                raise SystemExit(f"Missing declared license: {package['name']}")
            expressions.add(" ".join(expression.replace("/", " OR ").split()))
            crate = Path(package["manifest_path"]).parent
            destination = licenses / f"{package['name']}-{package['version']}"
            destination.mkdir()
            # Preserve upstream notices, including nested Unicode/third-party notices.
            retained = []
            for file in sorted(crate.rglob("*")):
                if file.is_file() and file.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "COPYRIGHT", "NOTICE", "AUTHORS", "README")):
                    relative = file.relative_to(crate)
                    target = destination / relative
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(file, target)
                    retained.append(str(relative))
            shutil.copyfile(crate / "Cargo.toml", destination / "Cargo.toml")
            inventory.append({"name": package["name"], "version": package["version"], "license": expression, "notices": retained})
        (licenses / "inventory.json").write_text(json.dumps(inventory, indent=2) + "\n")
        (source / "SOURCE_COMMIT").write_text(commit + "\n")
        (source / "SOURCE_DATE_EPOCH").write_text(epoch + "\n")
        shutil.copytree(source / "packaging/debian", source / "debian")
        changelog = source / "debian/changelog"
        lines = changelog.read_text().splitlines()
        lines[0] = f"epoxy-signing ({version}-1) unstable; urgency=medium"
        changelog.write_text("\n".join(lines) + "\n")
        # Conjunction of declared expressions is conservative (includes build and
        # non-Linux crates too). Preserve OR choices; never relabel them as MIT.
        combined = " AND ".join(f"({license})" for license in sorted(expressions | {"MIT"}))
        spec = (source / "packaging/fedora/epoxy-signing.spec.in").read_text().replace("@VERSION@", version).replace("@BUNDLED_LICENSES@", combined)
        (source / "packaging/fedora/epoxy-signing.spec").write_text(spec)
        archive = output / (name + ".tar.xz")
        env = os.environ | {"XZ_OPT": "-T1"}
        subprocess.run(["tar", "--sort=name", f"--mtime=@{epoch}", "--owner=0", "--group=0", "--numeric-owner", "-cJf", str(archive), "-C", str(temp), name], check=True, env=env)
        checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
        pkgbuild = (source / "packaging/arch/PKGBUILD.in").read_text().replace("@VERSION@", version).replace("@SHA256@", checksum).replace("@BUNDLED_LICENSES@", combined)
        (output / "PKGBUILD").write_text(pkgbuild)
        (output / "epoxy-signing.spec").write_text(spec)
        (output / "SHA256SUMS").write_text(f"{checksum}  {archive.name}\n")
        print(archive)


if __name__ == "__main__":
    main()
