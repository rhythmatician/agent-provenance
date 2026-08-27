from __future__ import annotations

import importlib.util
from pathlib import Path
import tempfile
import textwrap
import unittest

MODULE_PATH = Path(__file__).with_name("check_architecture.py")
SPEC = importlib.util.spec_from_file_location("check_architecture", MODULE_PATH)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class ArchitectureGuardrailTests(unittest.TestCase):
    def write_workspace(self, root: Path, core_dependency: str) -> None:
        members = {
            "provenance-domain": "",
            "provenance-core": core_dependency,
            "provenance-adapters": "provenance-domain = { path = '../provenance-domain' }\nprovenance-core = { path = '../provenance-core' }",
            "provenance-cli": "provenance-domain = { path = '../provenance-domain' }",
            "provenance-acceptance": "provenance-core = { path = '../../crates/provenance-core' }",
        }
        member_paths: list[str] = []
        for package, dependencies in members.items():
            if package == "provenance-acceptance":
                relative = "tests/provenance-acceptance"
            else:
                relative = f"crates/{package}"
            member_paths.append(relative)
            path = root / relative
            path.mkdir(parents=True)
            manifest = textwrap.dedent(
                f"""
                [package]
                name = "{package}"
                version = "0.1.0"
                edition = "2024"

                [dependencies]
                {dependencies}
                """
            ).strip()
            (path / "Cargo.toml").write_text(manifest + "\n", encoding="utf-8")

        quoted = ",\n    ".join(f'"{member}"' for member in member_paths)
        (root / "Cargo.toml").write_text(
            f"[workspace]\nmembers = [\n    {quoted}\n]\n",
            encoding="utf-8",
        )

    def test_allowed_dependency_direction_passes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.write_workspace(
                root,
                "provenance-domain = { path = '../provenance-domain' }",
            )
            self.assertEqual([], MODULE.check_architecture(root))

    def test_reverse_dependency_fails(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            self.write_workspace(
                root,
                "provenance-adapters = { path = '../provenance-adapters' }",
            )
            errors = MODULE.check_architecture(root)
            self.assertTrue(
                any(
                    "provenance-core must not depend on provenance-adapters" in error
                    for error in errors
                )
            )


if __name__ == "__main__":
    unittest.main()
