"""The skeleton lane S0s (R3) declared: every package imports, and ``ml/`` stays out of every cargo build.

Standard library only (``python3 -m unittest discover -s tests`` from ``ml/``; ``uv run`` once a lane has synced): no
dependency is needed to check the layout. Lanes DEG, AUG and TR add their own tests beside this file.
"""

import importlib
import pathlib
import tomllib
import unittest

ML = pathlib.Path(__file__).resolve().parents[1]
ROOT = ML.parent
PACKAGES = ["degrade", "augment", "model", "train", "export"]


class Layout(unittest.TestCase):
    def test_every_package_imports(self) -> None:
        importlib.import_module("enth_ml")
        for name in PACKAGES:
            with self.subTest(package=name):
                module = importlib.import_module(f"enth_ml.{name}")
                self.assertTrue(module.__doc__, f"enth_ml.{name} names its lane in its docstring")

    def test_the_project_declares_no_dependency_yet(self) -> None:
        project = tomllib.loads((ML / "pyproject.toml").read_text(encoding="utf-8"))
        self.assertEqual(project["project"]["name"], "enth-ml")
        self.assertEqual(project["project"]["dependencies"], [])

    def test_ml_is_in_no_cargo_build(self) -> None:
        self.assertEqual(list(ML.rglob("Cargo.toml")), [], "ml/ holds no Rust package")
        workspace = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["workspace"]
        self.assertIn("ml", workspace["exclude"], "the root manifest excludes ml/ from the workspace")
        for member in workspace["members"]:
            self.assertFalse(member == "ml" or member.startswith("ml/"), member)

    def test_ml_is_in_no_ci_build(self) -> None:
        ci = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
        # Both triggers (push and pull_request) ignore paths under ml/, so a Python-only change runs no workflow.
        self.assertEqual(ci.count('- "ml/**"'), 2, "ci.yml ignores ml/** on push and on pull_request")
        for workflow in (ROOT / ".github" / "workflows").glob("*.yml"):
            text = workflow.read_text(encoding="utf-8")
            self.assertNotIn("working-directory: ml", text, f"{workflow.name} builds in ml/")
            self.assertNotIn("uv sync", text, f"{workflow.name} syncs a Python environment")


if __name__ == "__main__":
    unittest.main()
