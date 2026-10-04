"""Local Win32 integration tests. Requires build-tray.ps1 -ResidentTest.

All windows, hooks and clipboard data live in a separate window station. Models
are reused by absolute path, and an owned job cleans up the entire test tree.
No GitHub Actions job invokes this script.
"""
import argparse
import ctypes as c
from ctypes import wintypes as w
import json
import msvcrt
import os
import re
from pathlib import Path
import shutil
import subprocess
import time
import uuid
import winreg

u = c.WinDLL('user32', use_last_error=True)
k = c.WinDLL('kernel32', use_last_error=True)
PTR = c.c_void_p
SIZE = c.c_size_t
LONG = c.c_ssize_t
CALLBACK = c.WINFUNCTYPE(LONG, PTR, w.UINT, SIZE, LONG)


def api(lib, name, result, *args):
    f = getattr(lib, name)
    f.restype, f.argtypes = result, args
    return f


class Startup(c.Structure):
    _fields_ = [('cb', w.DWORD), ('reserved', w.LPWSTR), ('desktop', w.LPWSTR),
                ('title', w.LPWSTR), ('x', w.DWORD), ('y', w.DWORD),
                ('cx', w.DWORD), ('cy', w.DWORD), ('charsx', w.DWORD),
                ('charsy', w.DWORD), ('fill', w.DWORD), ('flags', w.DWORD),
                ('show', w.WORD), ('bytes', w.WORD), ('data', PTR),
                ('stdin', PTR), ('stdout', PTR), ('stderr', PTR)]


class ProcessInfo(c.Structure):
    _fields_ = [('process', PTR), ('thread', PTR), ('pid', w.DWORD), ('tid', w.DWORD)]


class WindowClass(c.Structure):
    _fields_ = [('style', w.UINT), ('proc', CALLBACK), ('cls_extra', c.c_int),
                ('wnd_extra', c.c_int), ('instance', PTR), ('icon', PTR),
                ('cursor', PTR), ('background', PTR), ('menu', w.LPCWSTR),
                ('name', w.LPCWSTR)]


class CopyData(c.Structure):
    _fields_ = [('kind', SIZE), ('length', w.DWORD), ('data', PTR)]


class Entry(c.Structure):
    _fields_ = [('size', w.DWORD), ('uses', w.DWORD), ('pid', w.DWORD),
                ('heap', SIZE), ('module', w.DWORD), ('threads', w.DWORD),
                ('parent', w.DWORD), ('priority', w.LONG), ('flags', w.DWORD),
                ('name', w.WCHAR * 260)]


create_station = api(u, 'CreateWindowStationW', PTR, w.LPCWSTR, w.DWORD, w.DWORD, PTR)
set_station = api(u, 'SetProcessWindowStation', w.BOOL, PTR)
get_object_info = api(u, 'GetUserObjectInformationW', w.BOOL, PTR, c.c_int, PTR, w.DWORD, c.POINTER(w.DWORD))
create_desktop = api(u, 'CreateDesktopW', PTR, w.LPCWSTR, PTR, PTR, w.DWORD, w.DWORD, PTR)
set_desktop = api(u, 'SetThreadDesktop', w.BOOL, PTR)
create_process = api(k, 'CreateProcessW', w.BOOL, w.LPCWSTR, w.LPWSTR, PTR, PTR,
                     w.BOOL, w.DWORD, PTR, w.LPCWSTR, c.POINTER(Startup), c.POINTER(ProcessInfo))
close_handle = api(k, 'CloseHandle', w.BOOL, PTR)
wait_process = api(k, 'WaitForSingleObject', w.DWORD, PTR, w.DWORD)
process_exit = api(k, 'GetExitCodeProcess', w.BOOL, PTR, c.POINTER(w.DWORD))
open_process = api(k, 'OpenProcess', PTR, w.DWORD, w.BOOL, w.DWORD)
terminate = api(k, 'TerminateProcess', w.BOOL, PTR, w.UINT)
create_job = api(k, 'CreateJobObjectW', PTR, PTR, w.LPCWSTR)
set_job = api(k, 'SetInformationJobObject', w.BOOL, PTR, c.c_int, PTR, w.DWORD)
assign_job = api(k, 'AssignProcessToJobObject', w.BOOL, PTR, PTR)
module = api(k, 'GetModuleHandleW', PTR, w.LPCWSTR)
find_window = api(u, 'FindWindowW', PTR, w.LPCWSTR, w.LPCWSTR)
pid_window = api(u, 'GetWindowThreadProcessId', w.DWORD, PTR, c.POINTER(w.DWORD))
post = api(u, 'PostMessageW', w.BOOL, PTR, w.UINT, SIZE, LONG)
send_timeout = api(u, 'SendMessageTimeoutW', LONG, PTR, w.UINT, SIZE, LONG,
                   w.UINT, w.UINT, c.POINTER(SIZE))
get_text = api(u, 'GetWindowTextW', c.c_int, PTR, w.LPWSTR, c.c_int)
register = api(u, 'RegisterClassW', w.WORD, c.POINTER(WindowClass))
create_window = api(u, 'CreateWindowExW', PTR, w.DWORD, w.LPCWSTR, w.LPCWSTR, w.DWORD,
                    c.c_int, c.c_int, c.c_int, c.c_int, PTR, PTR, PTR, PTR)
destroy = api(u, 'DestroyWindow', w.BOOL, PTR)
default = api(u, 'DefWindowProcW', LONG, PTR, w.UINT, SIZE, LONG)
peek = api(u, 'PeekMessageW', w.BOOL, c.POINTER(w.MSG), PTR, w.UINT, w.UINT, w.UINT)
dispatch = api(u, 'DispatchMessageW', LONG, c.POINTER(w.MSG))
dlg_send = api(u, 'SendDlgItemMessageW', LONG, PTR, c.c_int, w.UINT, SIZE, LONG)
snapshot = api(k, 'CreateToolhelp32Snapshot', PTR, w.DWORD, w.DWORD)
first_process = api(k, 'Process32FirstW', w.BOOL, PTR, c.POINTER(Entry))
next_process = api(k, 'Process32NextW', w.BOOL, PTR, c.POINTER(Entry))
open_clipboard = api(u, 'OpenClipboard', w.BOOL, PTR)
close_clipboard = api(u, 'CloseClipboard', w.BOOL)
get_clipboard = api(u, 'GetClipboardData', PTR, w.UINT)
lock = api(k, 'GlobalLock', PTR, PTR)
unlock = api(k, 'GlobalUnlock', w.BOOL, PTR)


def checked(value):
    if not value:
        raise c.WinError(c.get_last_error())
    return value


def send(hwnd, message, wp=0, lp=0):
    result = SIZE()
    checked(send_timeout(hwnd, message, wp, lp, 2, 3000, c.byref(result)))
    return result.value


def text(hwnd):
    buf = c.create_unicode_buffer(512)
    get_text(hwnd, buf, len(buf))
    return buf.value


def pump():
    msg = w.MSG()
    while peek(c.byref(msg), None, 0, 0, 1):
        dispatch(c.byref(msg))


def until(predicate, timeout=15, description='condition'):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        pump()
        result = predicate()
        if result:
            return result
        time.sleep(.02)
    raise AssertionError('Timed out: ' + description)


def process_table():
    handle = snapshot(2, 0)
    rows = []
    entry = Entry()
    entry.size = c.sizeof(entry)
    try:
        more = first_process(handle, c.byref(entry))
        while more:
            rows.append((entry.pid, entry.parent, entry.name))
            more = next_process(handle, c.byref(entry))
    finally:
        close_handle(handle)
    return rows


def kill(pid):
    handle = checked(open_process(0x100001, False, pid))
    try:
        checked(terminate(handle, 1))
        assert wait_process(handle, 5000) == 0
    finally:
        close_handle(handle)


def window_pid(hwnd):
    pid = w.DWORD()
    checked(pid_window(hwnd, c.byref(pid)))
    return pid.value


def startup_value():
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER,
                        r'Software\Microsoft\Windows\CurrentVersion\Run') as key:
        try:
            return winreg.QueryValueEx(key, 'VTD Windows')
        except FileNotFoundError:
            return None


def main():
    args = argparse.ArgumentParser()
    args.add_argument('--build-dir', type=Path, default=Path(r'C:\vtd-build'))
    input_mode = args.add_mutually_exclusive_group(required=True)
    input_mode.add_argument('--wav', type=Path)
    input_mode.add_argument('--microphone', action='store_true')
    args.add_argument('--helper', type=Path, help='Use an exact production helper in the test resident')
    args.add_argument('--memory-script', type=Path)
    opts = args.parse_args()
    root = Path(__file__).resolve().parent.parent
    artifact = root / 'artifacts' / ('resident-test-' + uuid.uuid4().hex)
    artifact.mkdir(parents=True)
    base = root / 'dist' / 'vtd-windows'
    for name in ['vtd.exe', 'vtd-helper.exe']:
        source = opts.helper if name == 'vtd-helper.exe' and opts.helper else opts.build_dir / 'release' / name
        shutil.copy2(source, artifact / name)
    for name in ['vtd-engine.exe', 'vtd-transcribe.exe', 'msvcp140.dll',
                 'vcruntime140.dll', 'vcruntime140_1.dll']:
        os.link(base / name, artifact / name)
    cfg = json.loads((base / 'vtd.json').read_text(encoding='utf-8-sig'))
    cfg.update(model=str((base / cfg['model']).resolve()), idle_unload_seconds=1,
               mute_output=False, trigger_key=119, toggle_key=120, replay_key=121)
    config_path = artifact / 'vtd.json'
    config_path.write_text(json.dumps(cfg), encoding='utf-8')
    if opts.microphone:
        os.environ.pop('VTD_TEST_WAV', None)
    else:
        os.environ['VTD_TEST_WAV'] = str(opts.wav.resolve())
    original_startup = startup_value()
    station = checked(create_station(None, 0, 0xF037F, None))
    station_buf = c.create_unicode_buffer(256)
    required = w.DWORD()
    checked(get_object_info(station, 2, station_buf, c.sizeof(station_buf), c.byref(required)))
    station_name = station_buf.value
    checked(set_station(station))
    desktop = checked(create_desktop('Default', None, None, 0, 0xF01FF, None))
    checked(set_desktop(desktop))
    job = checked(create_job(None, None))
    limits = c.create_string_buffer(144)
    c.c_uint32.from_buffer(limits, 16).value = 0x2000
    checked(set_job(job, 9, limits, len(limits)))
    log = open(artifact / 'session.log', 'wb', buffering=0)
    os.set_handle_inheritable(msvcrt.get_osfhandle(log.fileno()), True)
    null = open(os.devnull, 'rb')
    os.set_handle_inheritable(msvcrt.get_osfhandle(null.fileno()), True)
    records = []
    handles = []

    def spawn(arguments):
        si = Startup()
        si.cb, si.desktop, si.flags = c.sizeof(si), station_name + r'\Default', 0x100
        si.stdin = msvcrt.get_osfhandle(null.fileno())
        si.stdout = si.stderr = msvcrt.get_osfhandle(log.fileno())
        pi = ProcessInfo()
        command = c.create_unicode_buffer(subprocess.list2cmdline([str(artifact / 'vtd.exe'), *arguments]))
        checked(create_process(str(artifact / 'vtd.exe'), command, None, None,
                               True, 0x8000000, None, str(artifact), c.byref(si), c.byref(pi)))
        checked(assign_job(job, pi.process))
        close_handle(pi.thread)
        handles.append(pi.process)
        return pi

    def cli(*arguments, expected=0):
        pi = spawn(arguments)
        assert wait_process(pi.process, 10000) == 0, arguments
        code = w.DWORD()
        checked(process_exit(pi.process, c.byref(code)))
        assert code.value == expected, (arguments, code.value)

    def passed(name):
        records.append(name)
        print('passed: ' + name, flush=True)
        (artifact / 'results.json').write_text(json.dumps(records, indent=2), encoding='utf-8')

    cache = ['']
    resident = None

    @CALLBACK
    def receive(hwnd, msg, wp, lp):
        if msg == 0x4A and resident and wp == resident.pid:
            data = c.cast(lp, c.POINTER(CopyData)).contents
            if data.kind == 0x56544403:
                cache[0] = c.string_at(data.data, data.length).decode('utf-8')
                return 1
        return default(hwnd, msg, wp, lp)

    try:
        instance = module(None)
        cls = WindowClass()
        cls.proc, cls.instance, cls.name = receive, instance, 'VTDIntegrationReader'
        checked(register(c.byref(cls)))
        reader = checked(create_window(0, cls.name, cls.name, 0, 0, 0, 0, 0, -3, None, instance, None))
        cli('--help')
        cli('check-config', str(config_path))
        cli('invalid-command', expected=1)
        cli('run', '--invalid', expected=1)
        cli('run', '--capture-next', expected=1)
        cli('run', '--capture-next', 'one.wav', 'two.wav', expected=1)
        passed('CLI forwarding and exit codes')
        capture_path = artifact / 'one shot.wav'
        resident = spawn(['run', '--capture-next', str(capture_path)])
        hwnd = until(lambda: find_window('VTDWindows', None), description='resident window')
        assert window_pid(hwnd) == resident.pid
        assert send(hwnd, 0x8015), 'test hooks require -ResidentTest'

        def children():
            return [(pid, name) for pid, parent, name in process_table() if parent == resident.pid]

        time.sleep(.5)
        until(lambda: not children(), description='bootstrap helper exits')
        if opts.memory_script:
            result = subprocess.run([os.sys.executable, str(opts.memory_script), str(resident.pid),
                                     '--seconds', '15', '--output', str(artifact / 'idle-memory.json')],
                                    capture_output=True, text=True, check=True)
            print(result.stdout, flush=True)
        cli('run')
        cli('status')
        assert not children()
        passed('bootstrap exits and singleton holds')

        def key(vk, down, modifiers=0):
            assert send(hwnd, 0x8014, vk, (modifiers << 1) | int(down)) == 1

        def tap(vk, modifiers=0):
            key(vk, True, modifiers)
            key(vk, False, modifiers)

        def last():
            cache[0] = ''
            send(hwnd, 0x8011, reader)
            return cache[0]

        if opts.microphone:
            started = time.monotonic()
            key(119, True)
            until(lambda: 'recording' in text(hwnd).lower(), description='live microphone opens')
            opened_ms = (time.monotonic() - started) * 1000
            time.sleep(.7)
            key(119, False)
            until(lambda: not children(), timeout=60, description='live recording and model release')
            output = (artifact / 'session.log').read_text(encoding='utf-8', errors='replace')
            match = re.search(r'VTD capture: ([0-9.]+)s audio, ([0-9.]+)s elapsed, first_packet=([0-9.]+)ms', output)
            assert match, output
            assert float(match[1]) >= .6 and float(match[3]) < 1000
            assert capture_path.stat().st_size > 16000 * 4 * .6
            passed('production helper opens real microphone, captures samples and releases all processes')
            cli('stop')
            assert wait_process(resident.process, 10000) == 0
            assert startup_value() == original_startup
            timings = {'shortcut_to_recording_ms': round(opened_ms, 2), 'audio_seconds': float(match[1]),
                       'first_packet_ms': float(match[3]), 'artifact': str(artifact)}
            (artifact / 'microphone.json').write_text(json.dumps(timings, indent=2), encoding='utf-8')
            print(json.dumps(timings), flush=True)
            return

        tap(119)  # Down and up both precede session startup.
        transcript = until(last, timeout=60, description='native model transcript')
        assert len(transcript) > 20, transcript
        until(lambda: not children(), timeout=15, description='recording helper exits after model unload')
        assert last() == transcript
        assert capture_path.is_file()
        capture_bytes = capture_path.read_bytes()
        passed('fast F8, real transcription, unload and resident transcript cache')
        tap(121)
        until(lambda: 'target window' in text(hwnd).lower(), description='F10 target guard')
        until(lambda: not children(), description='replay helper exits')
        assert last() == transcript
        passed('F10 survives helper exit and preserves target guard')
        cli('copy')
        checked(open_clipboard(reader))
        try:
            memory = checked(get_clipboard(13))
            pointer = checked(lock(memory))
            copied = c.wstring_at(pointer)
            unlock(memory)
        finally:
            close_clipboard()
        assert copied == transcript
        passed('CLI copy reads cached Unicode transcript without audio/model')
        cli('pause')
        for vk in [119, 120, 121]:
            tap(vk)
        time.sleep(.15)
        assert not children()
        assert text(hwnd) == 'VTD: paused'
        cli('resume')
        passed('pause suppresses all shortcuts')
        cli('settings')
        dialog = until(lambda: find_window('#32770', 'VTD - Settings'), description='settings dialog')
        for vk in [119, 120, 121]:
            tap(vk)
        time.sleep(.1)
        assert not find_window('VTDSession', None)
        for control, value in [(1001, 0x275), (1002, 0xAD), (1003, 0x376)]:
            dlg_send(dialog, control, 0x401, value, 0)
        dlg_send(dialog, 1006, 0xF1, 1, 0)
        send(dialog, 0x111, 1)
        until(lambda: not children(), description='settings helper exits after save')
        saved = json.loads(config_path.read_text(encoding='utf-8-sig'))
        assert (saved['trigger_key'], saved['toggle_key'], saved['replay_key']) == (0x275, 0xAD, 0x376)
        assert saved['mute_output'] and saved['language'] == cfg['language']
        assert startup_value() == original_startup
        passed('settings suspend shortcuts and save new keys/mute without altering startup')
        tap(119)
        tap(117)  # Missing Ctrl is rejected.
        time.sleep(.1)
        assert not children()
        key(117, True, 0x200)
        key(117, False, 0)  # Release works after the modifier changes.
        until(lambda: 'transcribing' in text(hwnd).lower(), description='custom Ctrl shortcut')
        until(lambda: not children(), timeout=60, description='custom recording finishes')
        assert last() == transcript
        passed('custom shortcuts match modifiers exactly and release correctly')
        assert capture_path.read_bytes() == capture_bytes
        assert (artifact / 'session.log').read_bytes().count(b'VTD diagnostic saved:') == 1
        passed('capture-next writes only the first recording')
        key(118, True, 0x300)
        key(118, False, 0x300)
        time.sleep(.15)
        assert not children(), 'replay must wait for modifier releases'
        key(0x11, False, 0x100)
        time.sleep(.1)
        assert not children()
        key(0x10, False, 0)
        until(lambda: 'target window' in text(hwnd).lower(), description='Ctrl+Shift replay')
        until(lambda: not children(), description='custom replay exits')
        passed('custom replay waits for all modifiers to release')
        tap(0xAD)
        until(lambda: text(hwnd).startswith('VTD: recording'), description='media-key toggle starts')
        cli('settings')
        time.sleep(.1)
        assert not find_window('#32770', 'VTD - Settings')
        assert find_window('VTDSession', None)
        passed('settings cannot interrupt an active recording')
        key(0xAD, True)
        key(0xAD, True)  # Auto-repeat must not stop it.
        time.sleep(.1)
        assert not find_window('#32770', 'VTD - Settings')
        key(0xAD, False)
        tap(0x1B)
        until(lambda: not children(), description='Escape cancels and releases model')
        assert last() == transcript
        passed('media key toggle, repeat suppression and Escape cancellation')
        cli('settings')
        dialog = until(lambda: find_window('#32770', 'VTD - Settings'))
        kill(window_pid(dialog))
        send(hwnd, 0x800D)  # The same check the hook posts after a GUI crash.
        key(117, True, 0x200)
        until(lambda: 'recording' in text(hwnd).lower(), description='shortcuts after GUI crash')
        session = checked(find_window('VTDSession', None))
        session_pid = window_pid(session)
        worker_pid = until(lambda: next((pid for pid, parent, name in process_table()
                                       if parent == session_pid and name.startswith('vtd-')), 0))
        worker = checked(open_process(0x100000, False, worker_pid))
        key(117, False)
        time.sleep(.04)
        kill(session_pid)
        assert wait_process(worker, 5000) == 0, 'model worker survived the helper crash'
        close_handle(worker)
        key(117, True, 0x200)
        until(lambda: find_window('VTDSession', None) and window_pid(find_window('VTDSession', None)) != session_pid,
              description='new session after crash')
        key(117, False)
        until(lambda: not children(), description='first hold/release after crash completes')
        assert last() == transcript
        passed('GUI/session crashes preserve shortcuts/cache and cannot orphan the model worker')
        # Replays can finish in milliseconds; new inputs must survive the idle handoff.
        for _ in range(20):
            tap(118, 0x300)
            key(0x10, False, 0)
            time.sleep(.012)
        until(lambda: not children(), description='rapid replays finish')
        assert last() == transcript
        key(117, True, 0x200)
        until(lambda: 'recording' in text(hwnd).lower(), description='recording after rapid handoffs')
        tap(0x1B)
        key(117, False)
        until(lambda: not children())
        passed('rapid session handoffs preserve the next shortcut')
        if opts.memory_script:
            result = subprocess.run([os.sys.executable, str(opts.memory_script), str(resident.pid),
                                     '--seconds', '15', '--output', str(artifact / 'used-memory.json')],
                                    capture_output=True, text=True, check=True)
            print(result.stdout, flush=True)
        cli('stop')
        assert wait_process(resident.process, 10000) == 0
        assert startup_value() == original_startup
        passed('clean exit')
        # The resident itself is also an owner: force-killing it must release its descendants.
        resident = spawn(['run'])
        hwnd = until(lambda: find_window('VTDWindows', None), description='restart for root crash')
        until(lambda: send(hwnd, 0x8015) != 0)
        time.sleep(.5)
        key(117, True, 0x200)
        session = until(lambda: find_window('VTDSession', None), description='session before root crash')
        session_pid = window_pid(session)
        worker_pid = until(lambda: next((pid for pid, parent, name in process_table()
                                       if parent == session_pid and name.startswith('vtd-')), 0))
        child_handles = [checked(open_process(0x100000, False, pid)) for pid in [session_pid, worker_pid]]
        kill(resident.pid)
        for handle in child_handles:
            assert wait_process(handle, 5000) == 0, 'descendant survived resident crash'
            close_handle(handle)
        passed('resident crash releases the recording helper and model process')
        print(json.dumps({'passed': len(records), 'artifact': str(artifact), 'transcript': transcript}, ensure_ascii=False), flush=True)
    finally:
        close_handle(job)
        for handle in handles:
            close_handle(handle)
        log.close()
        null.close()


if __name__ == '__main__':
    main()
