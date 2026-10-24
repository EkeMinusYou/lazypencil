"""Exercise the real terminal frontend without reading or changing the OS clipboard."""

import base64
import errno
import fcntl
import glob
import hashlib
import json
import math
import os
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import termios
import time
from terminal_screen import Screen
from terminal_images import KittyFrames

binary, root, fixture, case = sys.argv[1:]
environment = dict(os.environ)
document = os.path.join(root, "design.pen")
design = {"version": "2.21", "children": [{"type": "rectangle", "id": "red", "name": "日本語の対象", "x": -80, "y": -30, "width": 180, "height": 90, "fill": "#FF0000"}]}
if case == "dynamic-idle":
    with open(os.path.join(root, "generated.js"), "w") as handle:
        handle.write("/** @schema 2.21 */ return [{type:'rectangle',width:180,height:90,fill:'#00FF00'}];")
    with open(os.path.join(root, "blue.glsl"), "w") as handle:
        handle.write("precision highp float;\n/* @time */ uniform float clock;\nvoid main(){gl_FragColor=clock<0.5?vec4(0.,0.,1.,1.):vec4(1.,1.,0.,1.);}")
    design = {"version": "2.21", "children": [
        {"type": "script", "id": "script", "width": 180, "height": 90, "scriptUri": "generated.js"},
        {"type": "browser", "id": "page", "x": 180, "width": 180, "height": 90, "url": "data:text/html,<style>html{background:%23f00}</style>"},
        {"type": "rectangle", "id": "shader", "x": 360, "width": 180, "height": 90, "fill": {"type": "shader", "url": "blue.glsl"}},
    ]}
elif case in ["font-config", "user-font-config"]:
    font_root = os.path.join(os.path.dirname(os.path.dirname(fixture)), "fixtures", "fonts")
    configuration_path = os.path.join(root, "lazypencil.toml")
    if case == "user-font-config":
        environment["LAZYPENCIL_CONFIG_DIR"] = os.path.join(root, "user-settings")
        os.mkdir(environment["LAZYPENCIL_CONFIG_DIR"])
        configuration_path = os.path.join(environment["LAZYPENCIL_CONFIG_DIR"], "config.toml")
    with open(configuration_path, "w", encoding="utf-8") as handle:
        handle.write("version=1\n[fonts]\nfiles=[" + ",".join(json.dumps(os.path.join(font_root, name), ensure_ascii=False) for name in ["faces.ttc", "cff.otf"]) + "]\ndefault_family='LP Test Sans'\nlocale='en-US'\n")
    design = {"version": "2.21", "children": [{"type": "text", "id": "label", "content": "A", "fontSize": 80, "fill": "#FF0000"}]}
elif case == "script-config":
    configuration_path = os.path.join(root, "lazypencil.toml")
    configuration = "[runtime.javascript]\ntimeout_ms=100\nheap_mib=8\nidle_seconds=0\nlayout_timeout_ms=5000\nmax_evaluations=8\nmax_passes=4\n"
    with open(configuration_path, "w", encoding="utf-8") as handle:
        handle.write(configuration)
    with open(os.path.join(root, "generated.js"), "w", encoding="utf-8") as handle:
        handle.write("/** @schema 2.21 */ for (;;) {}")
    design = {"version": "2.21", "children": [{"type": "script", "id": "script", "width": 180, "height": 90, "scriptUri": "generated.js"}]}
elif case == "user-agent-config":
    environment["LAZYPENCIL_CONFIG_DIR"] = os.path.join(root, "user-settings")
    os.mkdir(environment["LAZYPENCIL_CONFIG_DIR"])
    configuration_path = os.path.join(environment["LAZYPENCIL_CONFIG_DIR"], "config.toml")
    with open(configuration_path, "w", encoding="utf-8") as handle:
        handle.write("[agents]\ndefault='claude'\n[agents.claude]\nadapter=" + json.dumps(sys.executable) + "\nadapter_args=[" + json.dumps(fixture) + ",'settings-auth-check']\nmodel='fixture-model-2'\nmode='workspace-write'\nauth_method='fixture-auth'\ntimeout=7\n")
elif case == "user-key-config":
    environment["LAZYPENCIL_CONFIG_DIR"] = os.path.join(root, "user-settings")
    os.mkdir(environment["LAZYPENCIL_CONFIG_DIR"])
    configuration_path = os.path.join(environment["LAZYPENCIL_CONFIG_DIR"], "config.toml")
    with open(configuration_path, "w", encoding="utf-8") as handle:
        handle.write("[tui.keys]\nsubmit=['F5']\npaste=[]\n")
with open(document, "w", encoding="utf-8") as handle:
    json.dump(design, handle)


def cli(*arguments):
    result = subprocess.run([binary, "--data-dir", os.path.join(root, "data"), "--json", *arguments], env=environment, capture_output=True, text=True)
    assert result.returncode == 0, result.stdout + result.stderr
    return json.loads(result.stdout)


def edit_color(color, name):
    revision = cli("inspect", document)["revision"]
    path = os.path.join(root, "operations.json")
    with open(path, "w") as handle:
        json.dump([{"op": "update", "target": "red", "set": {"fill": color}}], handle)
    return cli("apply", document, "--operations", path, "--expected-revision", revision, "--request-id", name)


if case in ["restore", "restore-conflict"]:
    first_checkpoint = edit_color("#22C55E", "first-checkpoint")
    edit_color("#2563EB", "second-checkpoint")
master, slave = pty.openpty()
original = termios.tcgetattr(slave)
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 34, 130, 0, 0))
image_case = "kitty" if case in ["dynamic-idle", "font-config", "user-font-config", "script-config"] else case.removeprefix("auto-")
protocol = "auto" if case.startswith("auto-") else (image_case if image_case in ["kitty", "sixel"] else "halfblocks")
command = [binary, "--data-dir", os.path.join(root, "data"), "tui", document,
    "--adapter", sys.executable, "--adapter-arg=" + fixture,
    "--adapter-arg=" + ("image-inspect-audit" if case == "basic" else "image-inspect"),
    "--image-protocol", protocol]
if case == "user-agent-config":
    command = [binary, "--data-dir", os.path.join(root, "data"), "tui", document,
        "--image-protocol", "halfblocks"]
output = bytearray()
kitty_frames = KittyFrames()
screen = Screen()
errors = open(os.path.join(root, "terminal-errors.log"), "wb")
process = subprocess.Popen(command, env=environment, cwd=root, stdin=slave, stdout=slave, stderr=errors, start_new_session=True)
probe_answered = False


def pump():
    global probe_answered
    if select.select([master], [], [], 0.02)[0]:
        try:
            block = os.read(master, 65536)
            output.extend(block)
            kitty_frames.feed(block)
            screen.feed(block)
            if case in ["auto-kitty", "auto-sixel"] and not probe_answered and b"\x1b[5n" in output:
                reply = b"\x1b_Gi=31;OK\x1b\\\x1b[?1;2c" if image_case == "kitty" else b"\x1b[?1;2;4c"
                os.write(master, reply + b"\x1b[6;20;10t\x1b[0n")
                probe_answered = True
            assert len(output) < 64 * 1024 * 1024, "Unbounded terminal output"
        except OSError as error:
            if error.errno != errno.EIO:
                raise


def wait(predicate, label, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        pump()
        if predicate():
            return
        if process.poll() is not None:
            raise AssertionError(f"TUI exited before {label}, code={process.returncode}")
    pending = "unknown"
    if hasattr(termios, "FIONREAD"):
        try:
            pending = struct.unpack("I", fcntl.ioctl(slave, termios.FIONREAD, struct.pack("I", 0)))[0]
        except OSError:
            pass
    raise AssertionError("Timed out: " + label + f" (pending input bytes: {pending})\n" + screen.text())


def session():
    files = glob.glob(os.path.join(root, "data", "**", "session.json"), recursive=True)
    if not files:
        return None
    with open(files[0], encoding="utf-8") as handle:
        return json.load(handle)


def turns():
    values = []
    for path in glob.glob(os.path.join(root, "data", "**", "turn-*.jsonl"), recursive=True):
        with open(path, encoding="utf-8") as handle:
            for line in handle:
                try:
                    values.append(json.loads(line))
                except json.JSONDecodeError:
                    pass
    return values


def paste(text):
    os.write(master, b"\x1b[200~" + text.encode("utf-8") + b"\x1b[201~")


def kitty_images():
    return iter(kitty_frames.images)


def sixel_red_pixels(data):
    """Independently decode RGB Sixel bands, repeats and palette changes."""
    match = re.search(rb'"\d+;\d+;(\d+);(\d+)', data)
    assert match, "Missing Sixel raster dimensions"
    width, height = map(int, match.groups())
    assert 0 < width <= 8192 and 0 < height <= 8192
    pixels = bytearray(width * height)
    colors = {}
    color = 0
    x = y = index = 0
    while index < len(data):
        symbol = data[index]
        if symbol in [ord('"'), ord('#')]:
            end = index + 1
            while end < len(data) and (48 <= data[end] <= 57 or data[end] == ord(';')):
                end += 1
            values = [int(value) for value in data[index + 1:end].split(b';') if value]
            if symbol == ord('#'):
                color = values[0]
                if len(values) == 5:
                    assert values[1] == 2, "Expected RGB Sixel palette"
                    colors[color] = tuple(round(value * 255 / 100) for value in values[2:])
            index = end
            continue
        repeat = 1
        if symbol == ord('!'):
            end = index + 1
            while end < len(data) and 48 <= data[end] <= 57:
                end += 1
            repeat = int(data[index + 1:end])
            assert 0 < repeat <= width
            symbol = data[end]
            index = end
        if 63 <= symbol <= 126:
            bits = symbol - 63
            assert x + repeat <= width
            for bit in range(6):
                if bits & (1 << bit) and y + bit < height:
                    red = colors.get(color) == (255, 0, 0)
                    offset = (y + bit) * width + x
                    pixels[offset:offset + repeat] = bytes([int(red)]) * repeat
            x += repeat
        elif symbol == ord('$'):
            x = 0
        elif symbol == ord('-'):
            x = 0
            y += 6
        index += 1
    return sum(pixels), width * height


try:
    wait(lambda: (b"F5 sends" if case == "user-key-config" else b"Ctrl+V") in output, "initial UI")
    if case == "user-key-config":
        text = "日本語の複数行\nq と hjkl は本文"
        paste(text)
        wait(lambda: session() and session()["draft"] == text, "custom-key multiline draft")
        # A later saved edit is a barrier proving Alt+Enter was processed.
        os.write(master, b"\x1b\r")
        paste(" 続けて入力")
        wait(lambda: session() and session()["draft"].endswith(" 続けて入力"), "input after replaced shortcut")
        assert not turns(), "The replaced key must not launch an agent"
        expected = session()["draft"]
        os.write(master, b"\x1b[15~")
        wait(lambda: any(value.get("type") == "complete" for value in turns()), "F5 submitted actual turn")
        received = json.load(open(os.path.join(root, "received-turn-1.json"), encoding="utf-8"))
        assert expected in received["text"], received
        assert not session()["draft"]
        # The invocation keeps its startup key bindings even if the file changes.
        with open(configuration_path, "w", encoding="utf-8") as handle:
            handle.write("[tui.keys]\nsubmit=['F6']\npaste=[]\n")
        paste("同じキーで次の依頼")
        os.write(master, b"\x1b[15~")
        wait(lambda: len([value for value in turns() if value.get("type") == "complete"]) == 2, "startup shortcut retained")
        assert len([value for value in turns() if value.get("type") == "ready"]) == 1
    elif case == "user-agent-config":
        assert session()["agent"] == "claude"
        assert not os.path.exists(os.path.join(root, "sent-turn.txt")), "Viewing must not launch a configured adapter"
        paste("保存したエージェント設定で送信")
        os.write(master, b"\x1b\r")
        wait(lambda: any(value.get("type") == "complete" for value in turns()), "configured agent completed turn")
        ready = next(value for value in turns() if value.get("type") == "ready")
        assert ready["response"]["modes"]["currentModeId"] == "workspace-write", ready
        assert ready["response"]["configOptions"][0]["currentValue"] == "fixture-model-2", ready
        # An existing live connection retains its invocation's resolved settings.
        with open(configuration_path, "w", encoding="utf-8") as handle:
            handle.write("[agents]\ndefault='codex'\n[agents.codex]\nadapter='missing/never-launch-this'\n")
        paste("同じ接続で次の依頼")
        os.write(master, b"\x13")
        wait(lambda: len([value for value in turns() if value.get("type") == "complete"]) == 2, "second turn kept resolved settings")
        starts = [value for value in turns() if value.get("type") == "start"]
        assert len(starts) == 2 and all(value["agent"] == "claude" for value in starts)
        assert len({value["session"] for value in starts}) == 1
        assert len([value for value in turns() if value.get("type") == "ready"]) == 1
        with open(document, encoding="utf-8") as handle:
            assert json.load(handle) == design
    elif case == "basic":
        text = "日本語の入力\nhjkl + - q は本文です"
        paste(text)
        wait(lambda: session() and session()["draft"] == text, "saved Japanese multiline draft")
        assert not os.path.exists(os.path.join(root, "sent-turn.txt")), "Draft must not send automatically"
        os.write(master, b"\x1b\r")
        wait(lambda: any(value.get("type") == "complete" for value in turns()), "first completed turn")
        wait(lambda: re.search(r"Request\s+complete", screen.text()), "visible turn completion")
        paste("次の依頼")
        os.write(master, b"\x13")
        wait(lambda: len([value for value in turns() if value.get("type") == "complete"]) == 2, "second completed turn")
        received = json.load(open(os.path.join(root, "received-turn-1.json"), encoding="utf-8"))
        assert text in received["text"]
        assert not session()["draft"]
        assert session()["native_session_id"] == "fixture-native"
        starts = [value for value in turns() if value.get("type") == "start"]
        usage = [value for value in turns() if value.get("type") == "design_resource"]
        assert len(starts) == 2 and len(usage) == 6
        assert len({value["connection"] for value in usage}) == 1, "Both turns must retain the same owned MCP connection"
        for number, guide_id in [(1, "typography"), (2, "color")]:
            with open(os.path.join(root, f"received-turn-{number}.json"), encoding="utf-8") as handle:
                resources = json.load(handle)["design_resources"]
            guides = [value for value in usage if value["tool"] == "get_guidance" and value["resources"][0]["id"] == guide_id]
            assert len(guides) == 1
            turn = guides[0]["turn"]
            turn_usage = [value for value in usage if value["turn"] == turn]
            assert len(turn_usage) == 3
            assert guides[0]["resources"][0]["hash"] == resources["guide_hash"]
            assert next(value for value in turn_usage if value["tool"] == "get_style")["resources"][0]["hash"] == resources["style_hash"]
            assert sum(value["turn"] == turn for value in starts) == 1
        # Pan and zoom outside Prompt. Resize while keeping the event loop responsive.
        os.write(master, b"\tll+0")
        for repetition in range(8):
            width = 145 + repetition % 2
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 38, width, 0, 0))
            screen.resize(width, 38)
            os.kill(process.pid, signal.SIGWINCH)
            os.write(master, b"\x1bOP")
            wait(lambda: "/clear-images" in screen.text(), f"help after resize {repetition}")
            os.write(master, b"\x1bOP")
            wait(lambda: "/clear-images" not in screen.text(), f"closed help {repetition}")
    elif case in ["restore", "restore-conflict"]:
        paste("復元しても保持する下書き")
        wait(lambda: session()["draft"] == "復元しても保持する下書き", "saved draft before restore")
        os.write(master, b"\x10")
        wait(lambda: "Commands" in screen.text() and "Restore checkpoint" in screen.text(), "command palette")
        os.write(master, b"\r")
        wait(lambda: "first-checkpoint" in screen.text() and "second-checkpoint" in screen.text(), "checkpoint candidates")
        os.write(master, b"\x1b[B\r")
        wait(lambda: "Restore selected checkpoint?" in screen.text(), "restoration review")
        assert json.load(open(document))["children"][0]["fill"] == "#2563EB", "Reviewing must not restore"
        os.write(master, b"\x1b")
        wait(lambda: "Checkpoints" in screen.text() and "Restore selected checkpoint?" not in screen.text(), "cancelled review")
        os.write(master, b"\r")
        wait(lambda: "Restore selected checkpoint?" in screen.text(), "review again")
        if case == "restore-conflict":
            external = edit_color("#D946EF", "external-edit")
            os.write(master, b"\r")
            wait(lambda: "REVISION_CONFLICT" in screen.text(), "restoration conflict")
            assert json.load(open(document))["children"][0]["fill"] == "#D946EF"
            assert len(cli("history", document)["entries"]) == 3
            os.write(master, b"r")
            wait(lambda: external["revision"][:12] in screen.text() and "Checkpoints" in screen.text(), "explicit revision refresh")
            os.write(master, b"\x1b[B\x1b[B\r")
            wait(lambda: "Restore selected checkpoint?" in screen.text() and "Checkpoint #1" in screen.text(), "fresh checkpoint review")
        os.write(master, b"\r")
        wait(lambda: json.load(open(document))["children"][0]["fill"] == "#22C55E", "restored document")
        wait(lambda: re.search(r"Checkpoint\s+restored", screen.text()), "visible completed restoration")
        assert session()["draft"] == "復元しても保持する下書き"
        history = cli("history", document)["entries"]
        assert len(history) == (4 if case == "restore-conflict" else 3)
        assert history[0]["result"]["restored_checkpoint"] == first_checkpoint["checkpoint_id"]
        assert not os.path.exists(os.path.join(root, "sent-turn.txt"))
    elif case == "auto-none":
        paste("最初の入力を保持する")
        wait(lambda: session()["draft"] == "最初の入力を保持する", "first input after probe timeout")
        assert not os.path.exists(os.path.join(root, "sent-turn.txt"))
    elif case == "dynamic-idle":
        saved = open(document, "rb").read()
        last_check = [0.0, []]

        def children():
            if time.monotonic() - last_check[0] >= 0.2:
                rows = subprocess.check_output(["ps", "-axo", "pid=,ppid="], text=True)
                last_check[:] = [time.monotonic(), [int(pid) for pid, parent in (line.split() for line in rows.splitlines()) if int(parent) == process.pid]]
            return last_check[1]

        def actual_dynamic_pixels(shader=(0, 0, 255, 255)):
            images = list(kitty_images())
            return images and all(bytes(color) in images[-1] for color in [(255, 0, 0, 255), (0, 255, 0, 255), shader])

        wait(lambda: len(children()) == 3 and actual_dynamic_pixels(), "three dynamic workers and actual RGB pixels")
        original_pids = children().copy()
        frame_count = len(list(kitty_images()))
        paste("/time 1")
        os.write(master, b"\r")
        wait(lambda: len(list(kitty_images())) > frame_count and bytes((255, 255, 0, 255)) in list(kitty_images())[-1] and "t=1.000s" in screen.text(), "time command changes real shader pixels")
        assert children() == original_pids, "Seeking must reuse the existing runtimes"
        assert session()["draft"] == ""
        frame_count = len(list(kitty_images()))
        # Two unchanged dependency polls must not repaint a paused clock.
        deadline = time.monotonic() + 2.2
        while time.monotonic() < deadline:
            pump()
        assert len(list(kitty_images())) == frame_count, "An unchanged time must not repaint"
        paste("/time 0")
        os.write(master, b"\r")
        wait(lambda: len(list(kitty_images())) > frame_count and actual_dynamic_pixels() and "t=0.000s" in screen.text(), "time command restores original shader pixels")
        assert children() == original_pids
        frame_count = len(list(kitty_images()))
        playback_started = time.monotonic()
        paste("/play 2")
        os.write(master, b"\r")

        def timeline():
            match = re.search(r"t=([0-9.]+)s", screen.text())
            return float(match[1]) if match else -1

        yellow = (255, 255, 0, 255)
        wait(lambda: timeline() >= 1.0 and "playing" in screen.text() and actual_dynamic_pixels(yellow), "playback advances real shader pixels and clock")
        paste("待機と更新の後も残る下書き")
        wait(lambda: session()["draft"] == "待機と更新の後も残る下書き", "Japanese draft preserved during playback")
        os.write(master, b"\t ")
        wait(lambda: "playing" not in screen.text().lower(), "Space pauses in Preview")
        assert children() == original_pids, "Automatic frames must not kill and restart the runtimes"
        assert len(list(kitty_images())) - frame_count <= math.ceil((time.monotonic() - playback_started) * 2) + 6, "Playback must respect the selected frame-rate cap"
        paused_time = timeline()
        paused_frames = len(list(kitty_images()))
        deadline = time.monotonic() + 2.2
        while time.monotonic() < deadline:
            pump()
        assert timeline() == paused_time, "Paused shader time must not advance"
        assert len(list(kitty_images())) <= paused_frames + 1, "Only an already requested frame may finish after pause"
        assert actual_dynamic_pixels(yellow)
        pids = children().copy()
        wait(lambda: not children(), "automatic idle process cleanup", seconds=75)
        for pid in pids:
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                pass
            else:
                raise AssertionError(f"Idle process {pid} was not reaped")
        assert actual_dynamic_pixels(yellow), "Preview must remain at its paused time after process cleanup"
        assert session()["draft"] == "待機と更新の後も残る下書き"
        frame_count = len(list(kitty_images()))
        os.write(master, b"\tr")
        wait(lambda: len(children()) == 3 and len(list(kitty_images())) > frame_count and actual_dynamic_pixels(yellow), "refresh after idle cleanup at paused time")
        assert all(pid not in pids for pid in children())
        assert open(document, "rb").read() == saved
        assert session()["draft"] == "待機と更新の後も残る下書き"
        assert not session()["native_session_id"], "Previewing must not start an agent"
    elif case == "script-config":
        saved = open(document, "rb").read()
        def status_line():
            return "".join(screen.cells[-2]).rstrip()
        wait(lambda: status_line().startswith("RENDER_TIMEOUT:"), "configured script execution timeout")
        paste("制限を変更しても残る下書き")
        wait(lambda: session()["draft"] == "制限を変更しても残る下書き", "draft after script timeout")
        configuration = configuration.replace("\ntimeout_ms=100\n", "\ntimeout_ms=2000\n")
        with open(configuration_path, "w", encoding="utf-8") as handle:
            handle.write(configuration)
        with open(os.path.join(root, "generated.js"), "w", encoding="utf-8") as handle:
            handle.write("/** @schema 2.21 */ let total=0;for(let i=0;i<5000000;i++)total+=i;return [{type:'rectangle',width:180,height:90,fill:total>0?'#00FF00':'#FF0000'}];")
        source_bytes = open(os.path.join(root, "generated.js"), "rb").read()
        def green_preview():
            return status_line().startswith("2 nodes · 0 diagnostics ·") and bool(list(kitty_images())) and bytes([0, 255, 0, 255]) in list(kitty_images())[-1]
        wait(green_preview, "recovered script preview")
        frames = len(list(kitty_images()))
        # Identical pixels need not produce another terminal image. Make the
        # policy change observable without changing the source or document.
        configuration = configuration.replace("heap_mib=8", "heap_mib=12")
        with open(configuration_path, "w", encoding="utf-8") as handle:
            handle.write(configuration.replace("\ntimeout_ms=2000\n", "\ntimeout_ms=1\n"))
        wait(lambda: status_line().startswith("RENDER_TIMEOUT:"), "policy-only timeout invalidates the preview")
        with open(configuration_path, "w", encoding="utf-8") as handle:
            handle.write(configuration)
        wait(lambda: len(list(kitty_images())) > frames and green_preview(), "policy-only repair restores the script preview")
        assert open(os.path.join(root, "generated.js"), "rb").read() == source_bytes
        def no_owned_children():
            rows = subprocess.check_output(["ps", "-axo", "pid=,ppid="], text=True)
            return not any(int(parent) == process.pid for pid, parent in (line.split() for line in rows.splitlines()))
        wait(no_owned_children, "zero-idle script worker cleanup")
        layout = cli("layout", document)
        assert layout["script_executions"][0]["limits"]["heap_mib"] == 12
        assert layout["script_executions"][0]["limits"]["timeout_ms"] == 2000
        assert layout["script_executions"][0]["limits"]["idle_seconds"] == 0
        assert session()["draft"] == "制限を変更しても残る下書き"
        assert not session()["native_session_id"]
        assert open(document, "rb").read() == saved
    elif case in ["font-config", "user-font-config"]:
        # VT diff updates may split a visible word across cursor commands. The
        # document path can also contain 'diagnostics'; inspect the status line.
        wait(lambda: bool(list(kitty_images())) and re.search(r"^1 nodes · 0 diagnostics ·", screen.text(), re.MULTILINE), "configured font preview")
        saved = open(document, "rb").read()
        before = list(kitty_images())[-1]
        paste("設定を変えても残る下書き")
        wait(lambda: session()["draft"] == "設定を変えても残る下書き", "saved draft before configuration update")
        frames = len(list(kitty_images()))
        path = configuration_path
        text = open(path, encoding="utf-8").read().replace("default_family='LP Test Sans'", "default_family='LP Test CFF'")
        with open(path, "w", encoding="utf-8") as handle:
            handle.write(text)
        try:
            wait(lambda: len(list(kitty_images())) > frames and list(kitty_images())[-1] != before, "automatic configuration/font reload")
        except AssertionError:
            images = list(kitty_images())
            print(json.dumps({
                "case": case,
                "initial_frames": frames,
                "final_frames": len(images),
                "before_pixels_sha256": hashlib.sha256(before).hexdigest(),
                "last_pixels_sha256": [hashlib.sha256(image).hexdigest() for image in images[-3:]],
                "configuration_text": open(path, encoding="utf-8").read(),
            }), flush=True)
            raise
        assert bytes([255, 0, 0, 255]) in list(kitty_images())[-1]
        assert session()["draft"] == "設定を変えても残る下書き"
        assert not session()["native_session_id"]
        assert open(document, "rb").read() == saved
    else:
        marker = b"\x1b_G" if image_case == "kitty" else b"\x1bP"
        wait(lambda: marker in output and (bool(re.search(rb'\x1b_G[^;]*m=0;[^\x1b]*\x1b\\', output)) if image_case == "kitty" else bool(re.search(rb'\x1bP[0-9;]*q.*?\x1b\\', output, re.DOTALL))), "complete encoded image")
        if image_case == "kitty":
            images = list(kitty_images())
            assert images, "No complete Kitty image"
            assert bytes([255, 0, 0, 255]) in images[0], "Actual red design pixels are required"
        else:
            frames = re.findall(rb'\x1bP[0-9;]*q(.*?)\x1b\\', bytes(output), re.DOTALL)
            assert frames, "No complete Sixel image"
            red, total = sixel_red_pixels(frames[-1])
            assert red > total // 4, "Decoded Sixel must contain the actual red design"
        assert not os.path.exists(os.path.join(root, "sent-turn.txt")), "Viewing must not launch the agent"
        assert not session()["native_session_id"]
    os.write(master, b"q" if case == "basic" else b"\x03")
    deadline = time.monotonic() + 10
    while process.poll() is None and time.monotonic() < deadline:
        pump()
    assert process.poll() == 0, f"TUI did not exit cleanly: {process.poll()}"
    while select.select([master], [], [], 0.05)[0]:
        pump()
    restored = termios.tcgetattr(slave)
    before_config, after_config = list(original), list(restored)
    if sys.platform == "darwin":
        # XNU sets this transient input-reprocessing bit when ICANON is restored,
        # including a plain Python setraw/tcsetattr round trip without this app.
        before_config[3] &= ~termios.PENDIN
        after_config[3] &= ~termios.PENDIN
    assert after_config == before_config, f"Terminal modes must be restored: before={original!r}, after={restored!r}"
    echo_start = len(output)
    os.write(master, b"mode-restored")
    assert not select.select([slave], [], [], 0)[0], "Canonical input must wait for Enter"
    os.write(master, b"\r")
    assert select.select([slave], [], [], 1)[0], "Enter must complete the restored input line"
    assert os.read(slave, 1024) == b"mode-restored\n", "Input translation must be restored"
    pump()
    assert b"mode-restored\r\n" in output[echo_start:], "Terminal echo and output translation must be restored"
    assert b"\x1b[?1049l" in output, "Alternate screen must be restored"
    if image_case == "kitty":
        assert re.search(rb"\x1b_Ga=d,d=I,i=\d+,q=2\x1b\\", output), "Delete only this app's Kitty resource"
        assert b"d=A" not in output
finally:
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait()
    errors.close()
    with open(os.path.join(root, "terminal-output.bin"), "wb") as handle:
        handle.write(output)
    with open(os.path.join(root, "terminal-screen.txt"), "w", encoding="utf-8") as handle:
        handle.write(screen.text())
    os.close(master)
    os.close(slave)

print(json.dumps({"case": case, "terminal_bytes": len(output), "exit_code": process.returncode}))
