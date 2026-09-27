"""Cancel lifecycle tests for the DeskUnlock pairing dialog.

The hardware regression this suite pins: pressing Cancel must reap the
whole pairing session (the ``syauth-device`` wrapper *and* the ``syauth
pair`` child) and close only the ``PairingDialog``, even when the Qt
event loop never delivers the fallback timer. The fallback therefore
runs on a plain ``threading.Timer`` that is independent of Qt.
"""

import importlib.machinery
import importlib.util
import os
import subprocess
import tempfile
import time
import unittest
from pathlib import Path
from unittest.mock import patch

from PySide6.QtWidgets import QApplication


MODULE_PATH = Path(__file__).parents[1] / "bin" / "syauth-settings"
loader = importlib.machinery.SourceFileLoader("syauth_settings", str(MODULE_PATH))
spec = importlib.util.spec_from_loader(loader.name, loader)
settings = importlib.util.module_from_spec(spec)
loader.exec_module(settings)

# Kept as a module global so the Qt application is not garbage collected.
APP = None


def setUpModule():
    """Create one offscreen QApplication for the QDialog/QProcess tests."""
    os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
    global APP
    APP = QApplication.instance() or QApplication([])


def pids_in_group(pgid):
    """Live (non-zombie) PIDs in process group ``pgid``."""
    pids = []
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/stat", "r") as handle:
                stat = handle.read()
        except OSError:
            continue
        close = stat.rfind(")")
        fields = stat[close + 2:].split()
        # fields[0] is the state; skip zombies/dead so a not-yet-reaped
        # child does not look like a survivor.
        if len(fields) >= 3 and fields[0] not in ("Z", "X") and int(fields[2]) == pgid:
            pids.append(int(entry))
    return pids


def wait_for(predicate, timeout=5.0):
    deadline = time.time() + timeout
    while time.time() < deadline:
        if predicate():
            return True
        time.sleep(0.02)
    return predicate()


class PairingDialogTestCase(unittest.TestCase):
    """Shared fixture: a temporary executable wrapper + dialog reaper."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)

    def make_wrapper(self, body):
        path = Path(self._tmp.name) / "syauth-device"
        path.write_text("#!/usr/bin/env bash\nset -u\n" + body + "\n")
        path.chmod(0o755)
        return path

    def make_dialog(self, wrapper):
        with patch.object(settings, "DEVICE_CONTROL", wrapper):
            dialog = settings.PairingDialog(None, "pair")
        self.addCleanup(self._reap, dialog)
        return dialog

    @staticmethod
    def _reap(dialog):
        dialog._cancel_watchdog_timer()
        process = dialog.process
        if process is None:
            return
        pid = process.processId()
        settings.kill_process_group(pid)
        process.waitForFinished(500)


class PairingDialogLifecycleTests(PairingDialogTestCase):
    def test_cancel_sends_one_command_and_arms_the_watchdog(self):
        dialog = self.make_dialog(self.make_wrapper("sleep 300 & sleep 300"))
        self.assertTrue(dialog.process.waitForStarted(5000))
        sent = []
        dialog.send = lambda command: sent.append(command)

        dialog.cancel_pairing()

        self.assertEqual(sent, ["cancel"])
        self.assertEqual(dialog.cancel.text(), "Annullamento…")
        self.assertFalse(dialog.cancel.isEnabled())
        self.assertTrue(dialog.cancel_requested)
        self.assertIsNotNone(dialog.cancel_watchdog)

    def test_repeated_cancel_sends_one_command_and_one_watchdog(self):
        dialog = self.make_dialog(self.make_wrapper("sleep 300 & sleep 300"))
        self.assertTrue(dialog.process.waitForStarted(5000))
        sent = []
        dialog.send = lambda command: sent.append(command)

        dialog.cancel_pairing()
        first_watchdog = dialog.cancel_watchdog
        dialog.cancel_pairing()
        dialog.cancel_pairing()

        self.assertEqual(sent, ["cancel"])
        self.assertIs(dialog.cancel_watchdog, first_watchdog)

    def test_cleanup_cancels_the_watchdog(self):
        dialog = self.make_dialog(self.make_wrapper("sleep 300 & sleep 300"))
        self.assertTrue(dialog.process.waitForStarted(5000))
        dialog.send = lambda command: None
        dialog.cancel_pairing()
        watchdog = dialog.cancel_watchdog
        self.assertIsNotNone(watchdog)

        dialog.cleanup_process()

        self.assertIsNone(dialog.cancel_watchdog)
        self.assertTrue(watchdog.finished.is_set())

    def test_watchdog_kill_targets_only_the_captured_pid(self):
        dialog = self.make_dialog(self.make_wrapper("sleep 300 & sleep 300"))
        self.assertTrue(dialog.process.waitForStarted(5000))
        pid = dialog.process.processId()
        # Calling the watchdog from the main thread would deliver the
        # signal synchronously (same-thread direct connection) and trigger
        # a second cleanup kill. Isolate the kill call.
        dialog.cancel_watchdog_fired.disconnect(dialog._on_cancel_watchdog_fired)
        killed = []
        with patch.object(
            settings,
            "kill_process_group",
            side_effect=lambda target: killed.append(target),
        ):
            dialog._watchdog_kill(pid)
        self.assertEqual(killed, [pid])

    def test_watchdog_fired_closes_only_the_dialog(self):
        dialog = self.make_dialog(self.make_wrapper("sleep 300 & sleep 300"))
        self.assertTrue(dialog.process.waitForStarted(5000))
        dialog.send = lambda command: None
        dialog.cancel_pairing()

        dialog._on_cancel_watchdog_fired()

        self.assertIsNone(dialog.process)
        self.assertEqual(dialog.result(), settings.QDialog.DialogCode.Rejected)

    def test_cancel_watchdog_keeps_the_parent_window_open(self):
        parent = settings.QWidget()
        parent.show()
        self.addCleanup(parent.deleteLater)
        with patch.object(settings, "DEVICE_CONTROL", self.make_wrapper("sleep 300 & sleep 300")):
            dialog = settings.PairingDialog(parent, "pair")
        self.addCleanup(self._reap, dialog)
        dialog.show()
        self.assertTrue(dialog.process.waitForStarted(5000))
        dialog.send = lambda command: None
        dialog.cancel_pairing()

        dialog._on_cancel_watchdog_fired()

        self.assertFalse(dialog.isVisible())
        self.assertTrue(
            parent.isVisible(),
            "cancel must close only PairingDialog, never the main window",
        )

    def test_bonded_shows_fine_and_hides_actions(self):
        dialog = self.make_dialog(self.make_wrapper("sleep 300"))

        dialog.apply_event({"event": "bonded"})

        self.assertEqual(dialog.status.text(), "Telefono associato")
        self.assertFalse(dialog.finish.isHidden())
        self.assertTrue(dialog.cancel.isHidden())
        self.assertTrue(dialog.confirm.isHidden())
        self.assertTrue(dialog.reject.isHidden())

    def test_fresh_dialog_shows_no_confirmation_state(self):
        """No ConfirmationRequired before a real request exists."""
        dialog = self.make_dialog(self.make_wrapper("sleep 300"))

        self.assertEqual(dialog.status.text(), "Preparazione di DeskUnlock…")
        self.assertTrue(dialog.confirm.isHidden())
        self.assertTrue(dialog.reject.isHidden())
        self.assertTrue(dialog.code.isHidden())
        self.assertTrue(dialog.code.isHidden())

    def test_lesc_code_shows_the_transport_confirmation(self):
        dialog = self.make_dialog(self.make_wrapper("sleep 300"))

        dialog.apply_event({"event": "lesc_code", "code": "123456"})

        self.assertEqual(dialog.code.text(), "123456")
        self.assertFalse(dialog.code.isHidden())
        self.assertFalse(dialog.confirm.isHidden())
        self.assertFalse(dialog.reject.isHidden())
        self.assertIn("Bluetooth", dialog.status.text())
        # A transport confirmation is not a success state.
        self.assertNotEqual(dialog.status.text(), "Telefono associato")

    def test_oob_ready_shows_the_deskunlock_confirmation(self):
        dialog = self.make_dialog(self.make_wrapper("sleep 300"))

        dialog.apply_event({"event": "oob_ready", "code": "04231789"})

        self.assertIn("04231789", dialog.code.text())
        self.assertFalse(dialog.code.isHidden())
        self.assertFalse(dialog.confirm.isHidden())
        self.assertFalse(dialog.reject.isHidden())
        self.assertNotEqual(dialog.status.text(), "Telefono associato")

    def test_error_event_never_claims_success(self):
        dialog = self.make_dialog(self.make_wrapper("sleep 300"))

        dialog.apply_event({"event": "error", "message": "associazione non riuscita"})

        self.assertEqual(dialog.status.text(), "associazione non riuscita")
        self.assertNotEqual(dialog.status.text(), "Telefono associato")
        self.assertTrue(dialog.confirm.isHidden())
        self.assertTrue(dialog.reject.isHidden())


class CancelWithoutEventLoopTests(PairingDialogTestCase):
    def test_cancel_kills_wrapper_and_child_without_qt_event_loop(self):
        """Hardware reproduction: the fallback must not need the Qt event loop.

        ``cancel_pairing`` is called and then NO Qt events are processed
        while waiting past the grace period. The old QTimer-based fallback
        could never fire here; the threading watchdog must.
        """
        if not os.path.isdir("/proc"):
            self.skipTest("no /proc on this host")
        dialog = self.make_dialog(self.make_wrapper("sleep 300 & sleep 300"))
        self.assertTrue(dialog.process.waitForStarted(5000))
        pid = dialog.process.processId()
        self.assertTrue(
            wait_for(lambda: len(pids_in_group(pid)) >= 2),
            "expected a wrapper + child process tree",
        )

        dialog.cancel_pairing()

        # Deliberately DO NOT process Qt events while waiting.
        self.assertTrue(
            wait_for(lambda: not pids_in_group(pid), timeout=5.0),
            "wrapper/child survived cancel without a Qt event loop",
        )

    def test_graceful_exit_before_timeout_cancels_watchdog_and_does_not_kill(self):
        dialog = self.make_dialog(self.make_wrapper("sleep 0.2"))
        self.assertTrue(dialog.process.waitForStarted(5000))
        dialog.send = lambda command: None

        dialog.cancel_pairing()
        watchdog = dialog.cancel_watchdog
        self.assertIsNotNone(watchdog)

        with patch.object(settings, "kill_process_group") as killer:
            self.assertTrue(dialog.process.waitForFinished(5000))
            dialog.finished(0, settings.QProcess.ExitStatus.NormalExit)
            self.assertIsNone(dialog.cancel_watchdog)
            self.assertTrue(watchdog.finished.is_set())
            killer.assert_not_called()
        self.assertEqual(
            dialog.result(),
            settings.QDialog.DialogCode.Rejected,
            "a cancelled pairing must leave the Annullamento state and close the dialog",
        )

    def test_watchdog_leaves_unrelated_processes_running(self):
        """The targeted group kill must never reach syauth-presenced."""
        if not os.path.isdir("/proc"):
            self.skipTest("no /proc on this host")
        unrelated = subprocess.Popen(["sleep", "300"], start_new_session=True)
        self.addCleanup(unrelated.wait, 5)
        self.addCleanup(unrelated.terminate)

        dialog = self.make_dialog(self.make_wrapper("sleep 300 & sleep 300"))
        self.assertTrue(dialog.process.waitForStarted(5000))
        pid = dialog.process.processId()
        self.assertTrue(wait_for(lambda: len(pids_in_group(pid)) >= 2))

        dialog.cancel_pairing()
        self.assertTrue(wait_for(lambda: not pids_in_group(pid), timeout=5.0))

        self.assertIsNone(
            unrelated.poll(),
            "unrelated process in another session must survive the pairing cleanup",
        )


class ProcessGroupHelperTests(unittest.TestCase):
    def test_new_session_parameters_request_a_private_session(self):
        parameters = settings.new_session_process_parameters()
        if parameters is None:
            self.skipTest("Qt build lacks UnixProcessParameters")
        self.assertTrue(parameters.flags & settings.QProcess.UnixProcessFlag.CreateNewSession)

    def test_terminate_process_tree_reaps_wrapper_and_child(self):
        if not os.path.isdir("/proc"):
            self.skipTest("no /proc on this host")
        parameters = settings.new_session_process_parameters()
        if parameters is None:
            self.skipTest("Qt build lacks UnixProcessParameters")

        process = settings.QProcess()
        process.setUnixProcessParameters(parameters)
        process.start("bash", ["-c", "sleep 300 & sleep 300"])
        self.assertTrue(process.waitForStarted(5000))
        pid = process.processId()
        self.assertTrue(wait_for(lambda: len(pids_in_group(pid)) >= 2))

        settings.terminate_process_tree(process)
        process.waitForFinished(5000)

        self.assertTrue(
            wait_for(lambda: not pids_in_group(pid), timeout=5.0),
            "process group still has live members after terminate_process_tree",
        )


if __name__ == "__main__":
    unittest.main()
