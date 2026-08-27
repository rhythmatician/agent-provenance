#!/usr/bin/env python3
"""Fail when an internal Cargo dependency violates the accepted layer direction."""

from __future__ import annotations

import argparse
from pathlib import Path
import sys
import tomllib
from typing import Any

ALLOWED_INTERNAL_DEPENDENCIES: dict[str, set[str]] = {
    "provenance-domain": set(),
    "provenance-core": {"provenance-domain"},
    "provenance-adapters": {"provenance-domain", "provenance-core"},
    "provenance-cli": {
        "provenance-domain",
        "provenance-core",
        "provenance-adapters",
    },
    "provenance-acceptance": {
        "provenance-domain",
        "provenance-core",
        "provenance-adapters",
        "provenance-cli",
    },
}

DEPENDENCY_SECTIONS = ("dependencies", "dev-dependencies", "build-dependencies")


def load_toml(path: Path) -> dict[str, Any]:
    with path.open("rb") as handle:
        return tomllib.load(handle)


def dependency_package_name(key: str, value: Any) -> str:
    if isinstance(value, dict) and isinstance(value.get("package"), str):
        return value["package"]
    return key


def collect_dependency_names(manifest: dict[str, Any]) -> set[str]:
    names: set[str] = set()
    for section_name in DEPENDENCY_SECTIONS:
        section = manifest.get(section_name, {})
        if isinstance(section, dict):
            for key, value in section.items():
                names.add(dependency_package_name(key, value))

    target = manifest.get("target", {})
    if isinstance(target, dict):
        for target_table in target.values():
            if not isinstance(target_table, dict):
                continue
            for section_name in DEPENDENCY_SECTIONS:
                section = target_table.get(section_name, {})
                if isinstance(section, dict):
                    for key, value in section.items():
                        names.add(dependency_package_name(key, value))
    return names


def workspace_manifest_paths(repo_root: Path) -> list[Path]:
    root_manifest = load_toml(repo_root / "Cargo.toml")
    members = root_manifest.get("workspace", {}).get("members", [])
    if not isinstance(members, list) or not all(isinstance(item, str) for item in members):
        raise ValueError("workspace.members must be a list of paths")
    return [repo_root / member / "Cargo.toml" for member in members]


def check_architecture(repo_root: Path) -> list[str]:
    manifests: dict[str, tuple[Path, dict[str, Any]]] = {}
    errors: list[str] = []

    for manifest_path in workspace_manifest_paths(repo_root):
        if not manifest_path.is_file():
            errors.append(f"missing workspace manifest: {manifest_path.relative_to(repo_root)}")
            continue
        manifest = load_toml(manifest_path)
        package_name = manifest.get("package", {}).get("name")
        if not isinstance(package_name, str):
            errors.append(f"manifest has no package.name: {manifest_path.relative_to(repo_root)}")
            continue
        manifests[package_name] = (manifest_path, manifest)

    actual_packages = set(manifests)
    policy_packages = set(ALLOWED_INTERNAL_DEPENDENCIES)
    for missing in sorted(policy_packages - actual_packages):
        errors.append(f"architecture policy names missing package: {missing}")
    for ungoverned in sorted(actual_packages - policy_packages):
        errors.append(f"workspace package has no architecture policy: {ungoverned}")

    internal_packages = actual_packages
    for package_name, (manifest_path, manifest) in manifests.items():
        allowed = ALLOWED_INTERNAL_DEPENDENCIES.get(package_name, set())
        actual_internal = collect_dependency_names(manifest) & internal_packages
        forbidden = actual_internal - allowed
        for dependency in sorted(forbidden):
            relative = manifest_path.relative_to(repo_root)
            errors.append(
                f"{package_name} must not depend on {dependency} ({relative})"
            )

        if package_name == "provenance-domain":
            all_dependencies = collect_dependency_names(manifest)
            if all_dependencies:
                errors.append(
                    "provenance-domain must remain dependency-free; found: "
                    + ", ".join(sorted(all_dependencies))
                )

    return errors


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "repo_root",
        nargs="?",
        type=Path,
        default=Path(__file__).resolve().parents[1],
    )
    args = parser.parse_args(argv)

    errors = check_architecture(args.repo_root.resolve())
    if errors:
        for error in errors:
            print(f"architecture violation: {error}", file=sys.stderr)
        return 1

    print("architecture dependency direction: pass")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
