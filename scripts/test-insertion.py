"""Local input regression test on an owned native Windows Edit control.

Unlike test-resident.py, this uses the interactive desktop and SendInput through
the real low-level hook. It stops if another application takes focus. The helper
restores the user's clipboard; the fixture restores the previous foreground
window on exit. Never run this in GitHub Actions.
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
import uuid

import importlib.util

spec = importlib.util.spec_from_file_location('resident_fixture', Path(__file__).with_name('test-resident.py'))
r = importlib.util.module_from_spec(spec)
spec.loader.exec_module(r)
api, checked = r.api, r.checked
u, k, PTR, SIZE, LONG = r.u, r.k, r.PTR, r.SIZE, r.LONG


class Keyboard(c.Structure):
    _fields_ = [('vk', w.WORD), ('scan', w.WORD), ('flags', w.DWORD),
                ('time', w.DWORD), ('extra', SIZE)]


class Mouse(c.Structure):
    _fields_ = [('dx', w.LONG), ('dy', w.LONG), ('data', w.DWORD),
                ('flags', w.DWORD), ('time', w.DWORD), ('extra', SIZE)]


class Hardware(c.Structure):
    _fields_ = [('message', w.DWORD), ('low', w.WORD), ('high', w.WORD)]


class Payload(c.Union):
    _fields_ = [('keyboard', Keyboard), ('mouse', Mouse), ('hardware', Hardware)]


class Input(c.Structure):
    _fields_ = [('type', w.DWORD), ('payload', Payload)]


class HookKeyboard(c.Structure):
    _fields_ = [('vk', w.DWORD), ('scan', w.DWORD), ('flags', w.DWORD),
                ('time', w.DWORD), ('extra', SIZE)]


send_input = api(u, 'SendInput', w.UINT, w.UINT, c.POINTER(Input), c.c_int)
foreground = api(u, 'GetForegroundWindow', PTR)
set_foreground = api(u, 'SetForegroundWindow', w.BOOL, PTR)
set_focus = api(u, 'SetFocus', PTR, PTR)
show = api(u, 'ShowWindow', w.BOOL, PTR, c.c_int)
translate = api(u, 'TranslateMessage', w.BOOL, c.POINTER(w.MSG))
async_state = api(u, 'GetAsyncKeyState', w.SHORT, c.c_int)
set_text = api(u, 'SetWindowTextW', w.BOOL, PTR, w.LPCWSTR)
HOOK = c.WINFUNCTYPE(LONG, c.c_int, SIZE, LONG)
install_hook = api(u, 'SetWindowsHookExW', PTR, c.c_int, HOOK, PTR, w.DWORD)
next_hook = api(u, 'CallNextHookEx', LONG, PTR, c.c_int, SIZE, LONG)
remove_hook = api(u, 'UnhookWindowsHookEx', w.BOOL, PTR)
enum_formats = api(u, 'EnumClipboardFormats', w.UINT, w.UINT)
format_name = api(u, 'GetClipboardFormatNameW', c.c_int, w.UINT, w.LPWSTR, c.c_int)


def pump():
    msg = w.MSG()
    while r.peek(c.byref(msg), None, 0, 0, 1):
        translate(c.byref(msg))
        r.dispatch(c.byref(msg))


def wait(predicate, timeout=15, description='condition'):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        pump()
        result = predicate()
        if result:
            return result
        time.sleep(.01)
    raise AssertionError('Timed out: ' + description)


def modifiers():
    return [vk for vk in [0xA0, 0xA1, 0xA2, 0xA3, 0xA4, 0xA5, 0x5B, 0x5C]
            if async_state(vk) < 0]


def clipboard_text(hwnd):
    checked(r.open_clipboard(hwnd))
    try:
        memory = r.get_clipboard(13)
        if not memory:
            return None
        pointer = checked(r.lock(memory))
        try:
            return c.wstring_at(pointer)
        finally:
            r.unlock(memory)
    finally:
        r.close_clipboard()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--build-dir', type=Path, default=Path(r'C:\vtd-build'))
    parser.add_argument('--wav', type=Path, required=True)
    parser.add_argument('--paste-limit', type=int, choices=[0, 1, 2, 3],
                        help='Exercise a partial SendInput and modifier cleanup')
    opts = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    base = root / 'dist' / 'vtd-windows'
    artifact = root / 'artifacts' / ('insertion-test-' + uuid.uuid4().hex)
    artifact.mkdir(parents=True)
    for name in ['vtd.exe', 'vtd-helper.exe']:
        shutil.copy2(opts.build_dir / 'release' / name, artifact / name)
    for name in ['vtd-engine.exe', 'vtd-transcribe.exe', 'msvcp140.dll',
                 'vcruntime140.dll', 'vcruntime140_1.dll']:
        os.link(base / name, artifact / name)
    cfg = json.loads((base / 'vtd.json').read_text(encoding='utf-8-sig'))
    cfg.update(model=str((base / cfg['model']).resolve()), idle_unload_seconds=1,
               mute_output=False, trigger_key=119, toggle_key=120, replay_key=121)
    (artifact / 'vtd.json').write_text(json.dumps(cfg), encoding='utf-8')
    assert not r.find_window('VTDWindows', None), 'Stop VTD before this interactive test'
    assert not modifiers(), 'Release modifier keys before running the test'
    previous = foreground()
    events, records = [], []
    log = open(artifact / 'session.log', 'wb', buffering=0)
    env = os.environ.copy()
    env['VTD_TEST_WAV'] = str(opts.wav.resolve())
    if opts.paste_limit is not None:
        env['VTD_TEST_PASTE_LIMIT'] = str(opts.paste_limit)
    else:
        env.pop('VTD_TEST_PASTE_LIMIT', None)
    resident = None
    fixture = edit = hook = None
    before_clipboard = None

    @r.CALLBACK
    def window(hwnd, message, wp, lp):
        if message == 7 and edit:
            set_focus(edit)
        return r.default(hwnd, message, wp, lp)

    @HOOK
    def observe(code, wp, lp):
        if code == 0:
            event = c.cast(lp, c.POINTER(HookKeyboard)).contents
            events.append({'vk': event.vk, 'message': wp, 'flags': event.flags,
                           'extra': event.extra, 'foreground': foreground() == fixture})
        return next_hook(None, code, wp, lp)

    def passed(name):
        records.append(name)
        print('passed: ' + name, flush=True)

    def input_key(vk, down):
        assert foreground() == fixture, 'Fixture lost focus; refusing to type elsewhere'
        event = Input(type=1, payload=Payload(keyboard=Keyboard(vk, 0, 0 if down else 2, 0, 0x56544449)))
        assert send_input(1, c.byref(event), c.sizeof(Input)) == 1
        deadline = time.monotonic() + .03
        wait(lambda: time.monotonic() >= deadline)

    def tap(vk):
        input_key(vk, True)
        input_key(vk, False)

    def children():
        return [pid for pid, parent, _ in r.process_table() if parent == resident.pid]

    def assert_restored():
        wait(lambda: not children(), description='clipboard restore and helper exit')
        assert clipboard_text(fixture) == before_clipboard, 'Clipboard was not restored'
        assert not modifiers(), 'A modifier key remained pressed'

    def assert_typing(expected):
        tap(0x56)
        wait(lambda: r.text(edit) == expected + 'v', description='ordinary v remains ordinary typing')
        assert not modifiers()

    try:
        instance = r.module(None)
        cls = r.WindowClass()
        cls.proc, cls.instance, cls.name = window, instance, 'VTDInsertionFixture'
        checked(r.register(c.byref(cls)))
        fixture = checked(r.create_window(0, cls.name, 'VTD input regression test',
                                         0x00CF0000, 200, 180, 600, 230, None, None, instance, None))
        edit = checked(r.create_window(0x200, 'EDIT', '', 0x50010004,
                                      12, 12, 555, 145, fixture, None, instance, None))
        show(fixture, 1)
        checked(set_foreground(fixture))
        set_focus(edit)
        hook = checked(install_hook(13, observe, instance, 0))
        before_clipboard = clipboard_text(fixture)
        resident = subprocess.Popen([str(artifact / 'vtd.exe'), 'run'], cwd=artifact,
                                    env=env, stdout=log, stderr=log)
        hwnd = wait(lambda: r.find_window('VTDWindows', None), description='resident')
        deadline = time.monotonic() + .5
        wait(lambda: time.monotonic() >= deadline)
        wait(lambda: not children(), description='bootstrap exit')
        assert r.send(hwnd, 0x8015), 'Build with -ResidentTest'
        tap(120)
        wait(lambda: 'recording' in r.text(hwnd).lower(), description='F9 recording')
        tap(120)
        wait(lambda: 'transcribing' in r.text(hwnd).lower(), description='F9 stops recording')
        wait(lambda: 'completed' in r.text(hwnd).lower() or 'Click the target' in r.text(hwnd),
             timeout=75, description='F9 transcription and insertion')
        print('F9 status: ' + r.text(hwnd), flush=True)
        if opts.paste_limit is not None:
            evidence = f'VTD test paste: requested={opts.paste_limit} sent={opts.paste_limit}'
            for attempt in range(3):
                assert_restored()
                output = (artifact / 'session.log').read_text(encoding='utf-8', errors='replace')
                if evidence in output:
                    break
                assert 'Clipboard is busy' in output, r.text(hwnd)
                tap(121)
            else:
                raise AssertionError('The forced partial SendInput was not reached')
            assert ('Windows did not allow all text' in output
                    or 'Clipboard is busy' in output), output
            assert_typing(r.text(edit))
            passed('partial Ctrl+V releases injected keys and restores the clipboard')
            return
        transcript = r.text(edit)
        assert len(transcript) > 20, 'F9 did not insert text: ' + r.text(hwnd)
        assert_restored()
        passed('F9 starts, stops, transcribes and pastes into a real Edit control')
        checked(set_text(edit, ''))
        set_focus(edit)
        tap(121)
        wait(lambda: r.text(edit) == transcript, description='F10 insertion after helper unload')
        assert_restored()
        assert_typing(transcript)
        passed('F10 replays after unload, restores clipboard and leaves ordinary typing intact')
        checked(set_text(edit, ''))
        set_focus(edit)
        # Replaying inside the 300 ms clipboard restoration interval must fail
        # gracefully, then work again without losing the transcript or keyboard.
        tap(121)
        wait(lambda: r.text(edit) == transcript, description='first rapid replay')
        tap(121)
        assert_restored()
        count = r.text(edit).count(transcript)
        assert count in (1, 2) and r.text(edit) == transcript * count
        tap(121)
        wait(lambda: r.text(edit) == transcript * (count + 1), description='replay after rapid keys')
        assert_restored()
        passed('rapid F10 presses cannot crash paste handling or leave Ctrl pressed')
        checked(set_text(edit, ''))
        set_focus(edit)
        input_key(119, True)
        wait(lambda: 'recording' in r.text(hwnd).lower(), description='F8 hold recording')
        input_key(119, False)
        wait(lambda: r.text(edit) == transcript, timeout=75, description='F8 release inserts transcript')
        assert_restored()
        passed('F8 hold/release also pastes the real transcript and releases its helper')
        # Ordinary typing during recognition must keep the cached result available
        # for F10 while preventing insertion into a target the user changed.
        checked(set_text(edit, ''))
        set_focus(edit)
        tap(120)
        wait(lambda: 'recording' in r.text(hwnd).lower())
        tap(120)
        wait(lambda: 'transcribing' in r.text(hwnd).lower())
        tap(0x56)
        wait(lambda: 'target window or field changed' in r.text(hwnd).lower(), timeout=75,
             description='typing invalidates the pending target')
        assert_restored()
        assert r.text(edit) == 'v'
        tap(121)
        wait(lambda: r.text(edit) == 'v' + transcript, description='F10 after target rejection')
        assert_restored()
        passed('typing while transcribing prevents stale insertion; F10 still recovers the result')
        # Configure Ctrl+Shift+F10 through the actual settings window.
        r.post(hwnd, 0x8006, 0, 0)
        dialog = wait(lambda: r.find_window('#32770', 'VTD - Settings'), description='settings')
        wait(lambda: r.dlg_send(dialog, 1003, 0x402, 0, 0) == 121,
             description='hotkey controls initialized')
        r.dlg_send(dialog, 1003, 0x401, 0x379, 0)
        assert r.dlg_send(dialog, 1003, 0x402, 0, 0) == 0x379
        r.send(dialog, 0x111, 1)
        wait(lambda: not children(), description='settings saved')
        assert json.loads((artifact / 'vtd.json').read_text(encoding='utf-8-sig'))['replay_key'] == 0x379
        checked(set_foreground(fixture))
        set_focus(edit)
        checked(set_text(edit, ''))
        for vk in [0xA2, 0xA0]:
            input_key(vk, True)
        tap(121)
        assert r.text(edit) == '', 'Replay ran before modifiers were released'
        input_key(0xA2, False)
        assert r.text(edit) == '', 'Replay ran while Shift was still down'
        input_key(0xA0, False)
        wait(lambda: r.text(edit) == transcript, description='modified F10 after releases')
        assert_restored()
        assert_typing(transcript)
        passed('Ctrl+Shift+F10 waits for modifier releases and leaves subsequent typing intact')
        r.send(hwnd, 0x10)
        resident.wait(timeout=10)
        print(json.dumps({'passed': len(records), 'artifact': str(artifact)}, ensure_ascii=False), flush=True)
    finally:
        if resident and resident.poll() is None:
            hwnd = r.find_window('VTDWindows', None)
            if hwnd:
                r.post(hwnd, 0x10, 0, 0)
                try:
                    wait(lambda: resident.poll() is not None, timeout=10, description='cleanup')
                except AssertionError:
                    resident.kill()
                    resident.wait(timeout=5)
        if hook:
            remove_hook(hook)
        if fixture:
            restore_focus = foreground() == fixture
            r.destroy(fixture)
            if restore_focus and previous:
                set_foreground(previous)
        log.close()
        (artifact / 'events.json').write_text(json.dumps(events, indent=2), encoding='utf-8')
        (artifact / 'results.json').write_text(json.dumps(records, indent=2), encoding='utf-8')
        print('Artifact: ' + str(artifact), flush=True)


if __name__ == '__main__':
    main()
