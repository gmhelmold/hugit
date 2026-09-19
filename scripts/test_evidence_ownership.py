#!/usr/bin/env python3
"""Evidence workspace ownership tests. All filesystem mutations use private fixtures."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import os
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest import mock

# An alternate source is used only by the retained red-before/green-after proof.
SOURCE = Path(os.environ.get("HUGIT_EVIDENCE_SOURCE", str(Path(__file__).with_name("evidence_report.py"))))
spec = importlib.util.spec_from_file_location("evidence_report_ownership_test", SOURCE)
assert spec and spec.loader
report = importlib.util.module_from_spec(spec)
spec.loader.exec_module(report)


class OwnershipTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="hugit-evidence-ownership-")
        self.root = Path(self.tmp.name)
        self.sentinel = self.root / "unrelated"
        self.sentinel.write_bytes(b"unrelated bytes\x00\xff")
        self.before = self.sentinel.read_bytes()
        self.output = self.root / "bag"
        self.work = self.root / "work"
        self.archive = self.root / "bag.tar"
        self.args = argparse.Namespace(hugit_bin=sys.executable, output=str(self.output),
                                      work_dir=str(self.work), empty_selected=False, run_id="ownership-test")

    def tearDown(self):
        self.assertEqual(self.sentinel.read_bytes(), self.before)
        self.tmp.cleanup()
        self.assertFalse(self.root.exists())

    def refuse_before_commands(self):
        with mock.patch.object(report.Driver, "run", side_effect=AssertionError("unexpected command")) as command:
            with self.assertRaises((OSError, ValueError)):
                report.build(self.args)
            command.assert_not_called()

    def test_existing_output_directory_is_not_reset(self):
        self.output.mkdir(); keep = self.output / "retained-evidence"
        keep.write_bytes(b"previous evidence")
        identity = self.output.stat().st_ino
        self.refuse_before_commands()
        self.assertEqual(keep.read_bytes(), b"previous evidence")
        self.assertEqual(self.output.stat().st_ino, identity)
        self.assertFalse(self.work.exists())

    def test_empty_output_directory_is_not_adopted(self):
        self.output.mkdir(); identity = self.output.stat().st_ino
        self.refuse_before_commands()
        self.assertEqual(self.output.stat().st_ino, identity)
        self.assertEqual(list(self.output.iterdir()), [])

    def test_output_file_is_preserved(self):
        self.output.write_bytes(b"not a directory")
        self.refuse_before_commands()
        self.assertEqual(self.output.read_bytes(), b"not a directory")

    @unittest.skipUnless(hasattr(os, "symlink"), "platform has no symlinks")
    def test_output_directory_symlink_is_preserved(self):
        self.output.symlink_to(self.root, target_is_directory=True)
        self.refuse_before_commands()
        self.assertTrue(self.output.is_symlink())
        self.assertEqual(self.output.readlink(), self.root)

    @unittest.skipUnless(hasattr(os, "symlink"), "platform has no symlinks")
    def test_dangling_output_link_is_preserved(self):
        missing = self.root / "missing-target"
        self.output.symlink_to(missing)
        self.refuse_before_commands()
        self.assertTrue(self.output.is_symlink())
        self.assertFalse(missing.exists())

    def test_existing_work_directory_is_preserved_before_output_creation(self):
        self.work.mkdir(); keep = self.work / "notes"
        keep.write_bytes(b"caller data")
        self.refuse_before_commands()
        self.assertEqual(keep.read_bytes(), b"caller data")
        self.assertFalse(self.output.exists())

    def test_existing_work_file_is_preserved(self):
        self.work.write_bytes(b"caller file")
        self.refuse_before_commands()
        self.assertEqual(self.work.read_bytes(), b"caller file")
        self.assertFalse(self.output.exists())

    @unittest.skipUnless(hasattr(os, "symlink"), "platform has no symlinks")
    def test_dangling_work_link_is_preserved(self):
        self.work.symlink_to(self.root / "absent-work")
        self.refuse_before_commands()
        self.assertTrue(self.work.is_symlink())
        self.assertFalse(self.output.exists())

    def test_equal_output_and_work_are_rejected(self):
        self.args.work_dir = str(self.output)
        self.refuse_before_commands()
        self.assertFalse(self.output.exists())

    def test_archive_and_work_collision_is_rejected(self):
        self.args.work_dir = str(self.archive)
        self.refuse_before_commands()
        self.assertFalse(self.output.exists())
        self.assertFalse(self.archive.exists())

    def test_work_inside_output_is_rejected_without_creating_parents(self):
        self.args.work_dir = str(self.output / "work")
        self.refuse_before_commands()
        self.assertFalse(self.output.exists())

    def test_output_inside_work_is_rejected_without_creating_parents(self):
        self.args.output = str(self.work / "bag")
        self.refuse_before_commands()
        self.assertFalse(self.work.exists())

    def test_missing_output_parent_is_not_created(self):
        self.args.output = str(self.root / "missing" / "bag")
        self.refuse_before_commands()
        self.assertFalse((self.root / "missing").exists())
        self.assertFalse(self.work.exists())

    def test_previous_archive_is_preserved_before_work_starts(self):
        self.archive.write_bytes(b"previous archive")
        self.refuse_before_commands()
        self.assertEqual(self.archive.read_bytes(), b"previous archive")
        self.assertFalse(self.output.exists())
        self.assertFalse(self.work.exists())

    @unittest.skipUnless(hasattr(os, "symlink"), "platform has no symlinks")
    def test_archive_symlink_is_not_followed_or_removed(self):
        self.archive.symlink_to(self.sentinel)
        self.refuse_before_commands()
        self.assertTrue(self.archive.is_symlink())
        self.assertFalse(self.output.exists())

    def test_failure_keeps_partial_evidence_and_removes_only_owned_work(self):
        def stop(*_args, **_kwargs):
            (self.output / "data/commands/observed.stdout").write_bytes(b"retained failure")
            raise RuntimeError("controlled command failure")
        with mock.patch.object(report.Driver, "git", side_effect=stop):
            with self.assertRaisesRegex(RuntimeError, "controlled command failure"):
                report.build(self.args)
        self.assertFalse(self.work.exists())
        self.assertEqual((self.output / "data/commands/observed.stdout").read_bytes(), b"retained failure")
        self.assertFalse((self.output / "data/report.json").exists())
        self.assertFalse(self.archive.exists())

    def test_default_temporary_work_is_removed_after_failure(self):
        self.args.work_dir = None
        made = []
        real_mkdtemp = tempfile.mkdtemp
        def owned(**kwargs):
            path = real_mkdtemp(dir=self.root, **kwargs); made.append(Path(path)); return path
        with mock.patch.object(report.tempfile, "mkdtemp", side_effect=owned), \
             mock.patch.object(report.Driver, "git", side_effect=RuntimeError("controlled stop")):
            with self.assertRaisesRegex(RuntimeError, "controlled stop"):
                report.build(self.args)
        self.assertEqual(len(made), 1)
        self.assertFalse(made[0].exists())
        self.assertTrue(self.output.is_dir())

    def test_preflight_race_cannot_overwrite_new_output_occupant(self):
        self.args.work_dir = None
        real_mkdtemp = tempfile.mkdtemp
        made = []
        def collide(**kwargs):
            self.output.mkdir(); (self.output / "new-owner").write_bytes(b"preserve")
            path = real_mkdtemp(dir=self.root, **kwargs); made.append(Path(path)); return path
        with mock.patch.object(report.tempfile, "mkdtemp", side_effect=collide):
            self.refuse_before_commands()
        self.assertEqual((self.output / "new-owner").read_bytes(), b"preserve")
        self.assertEqual(len(made), 1)
        self.assertFalse(made[0].exists())

    def test_replaced_work_directory_is_not_removed(self):
        saved = self.root / "original-work"
        def replace(*_args, **_kwargs):
            self.work.rename(saved)
            self.work.mkdir(); (self.work / "new-owner").write_bytes(b"preserve")
            raise RuntimeError("controlled stop")
        with mock.patch.object(report.Driver, "git", side_effect=replace):
            with self.assertRaisesRegex(RuntimeError, "replaced"):
                report.build(self.args)
        self.assertEqual((self.work / "new-owner").read_bytes(), b"preserve")
        self.assertTrue(saved.is_dir())

    @unittest.skipUnless(hasattr(os, "symlink"), "platform has no symlinks")
    def test_replaced_work_symlink_is_not_followed_by_cleanup(self):
        saved = self.root / "original-work"
        def replace(*_args, **_kwargs):
            self.work.rename(saved); self.work.symlink_to(self.root, target_is_directory=True)
            raise RuntimeError("controlled stop")
        with mock.patch.object(report.Driver, "git", side_effect=replace):
            with self.assertRaisesRegex(RuntimeError, "original identity"):
                report.build(self.args)
        self.assertTrue(self.work.is_symlink())
        self.assertTrue(saved.is_dir())

    @unittest.skipUnless(hasattr(os, "symlink"), "platform has no symlinks")
    def test_parent_alias_cannot_hide_output_work_collision(self):
        parent = self.root / "actual"
        parent.mkdir()
        alias = self.root / "alias"
        alias.symlink_to(parent, target_is_directory=True)
        self.args.output = str(alias / "same")
        self.args.work_dir = str(parent / "same")
        self.refuse_before_commands()
        self.assertEqual(list(parent.iterdir()), [])
        self.assertTrue(alias.is_symlink())

    @unittest.skipUnless(hasattr(os, "symlink"), "platform has no symlinks")
    def test_parent_dotdot_keeps_filesystem_resolution_semantics(self):
        actual = self.root / "actual" / "child"
        actual.mkdir(parents=True)
        alias = self.root / "alias"
        alias.symlink_to(actual, target_is_directory=True)
        destination = report.new_evidence_path(str(alias / ".." / "new-output"))
        # Resolve the fixture ancestor too: macOS /tmp is an alias of /private/tmp.
        self.assertEqual(destination, actual.parent.resolve(strict=True) / "new-output")
        self.assertFalse(destination.exists())

    def test_cleanup_removes_owned_tree(self):
        self.work.mkdir(); identity = report.directory_identity(self.work)
        (self.work / "nested").mkdir(); (self.work / "nested/data").write_bytes(b"own data")
        report.remove_owned_work(self.work, identity)
        self.assertFalse(self.work.exists())

    def test_cleanup_failure_is_not_silenced(self):
        self.work.mkdir(); identity = report.directory_identity(self.work)
        with mock.patch.object(report.shutil, "rmtree", side_effect=OSError("controlled cleanup error")):
            with self.assertRaisesRegex(OSError, "controlled cleanup error"):
                report.remove_owned_work(self.work, identity)
        self.assertTrue(self.work.is_dir())

    def test_archive_refuses_existing_bytes(self):
        self.output.mkdir(); (self.output / "payload").write_bytes(b"new data")
        self.archive.write_bytes(b"retained archive")
        identity = self.archive.stat().st_ino
        with self.assertRaises(FileExistsError):
            report.archive_bag(self.output)
        self.assertEqual(self.archive.read_bytes(), b"retained archive")
        self.assertEqual(self.archive.stat().st_ino, identity)

    def test_fresh_archive_preserves_deterministic_format(self):
        self.output.mkdir(); (self.output / "payload").write_bytes(b"retained data")
        report.archive_bag(self.output)
        other = self.root / "other.tar"
        report.safe_directory_tar(self.output, other)
        self.assertEqual(hashlib.sha256(self.archive.read_bytes()).digest(), hashlib.sha256(other.read_bytes()).digest())
        with tarfile.open(self.archive) as archive:
            self.assertEqual(archive.getnames(), ["payload"])
            member = archive.getmember("payload")
            self.assertEqual((member.uid, member.gid, member.mtime, member.mode), (0, 0, 0, 0o644))
            self.assertEqual(archive.extractfile(member).read(), b"retained data")

    @unittest.skipUnless(hasattr(os, "symlink"), "platform has no symlinks")
    def test_archive_rejects_source_link_before_creating_archive(self):
        self.output.mkdir(); (self.output / "link").symlink_to(self.sentinel)
        with self.assertRaisesRegex(RuntimeError, "unsafe repository node"):
            report.archive_bag(self.output)
        self.assertFalse(self.archive.exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
