#!/usr/bin/env python3
"""Drive the focus-board page in a headless Firefox through Marionette, against a scratch board.

Usage: python3 -I browser-check.py URL OUT_DIR [MARIONETTE_PORT]

Expects the board behind URL to hold, at least: B1 with options a/b (a fork's feedback on it
containing the literal text "<b>not bold</b>"), and B3 with options, in a project folder named
"beta". Loads the page, checks the table, tree and timeline render, answers B1 through the
page's own controls (a real browser POST: the browser sets Origin itself), provokes a stale save
on B3, narrows the table by a tree node, and saves two screenshots. Uses a throwaway Firefox
profile; never the owner's. Exit 0 when every check passed.
"""
import base64
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time

url, out = sys.argv[1], os.path.abspath(sys.argv[2])  # file:// links need an absolute path
port = int(sys.argv[3]) if len(sys.argv) > 3 else 31828
os.makedirs(out, exist_ok=True)
profile = tempfile.mkdtemp(prefix="fb-firefox-")
prefs = {
    "marionette.port": port,
    "browser.shell.checkDefaultBrowser": False,
    "datareporting.policy.dataSubmissionEnabled": False,
    "toolkit.telemetry.enabled": False,
    "app.update.enabled": False,
    "browser.startup.homepage_override.mstone": "ignore",
}
with open(os.path.join(profile, "user.js"), "w") as f:
    for k, v in prefs.items():
        f.write(f"user_pref({json.dumps(k)}, {json.dumps(v)});\n")
firefox = subprocess.Popen(
    ["firefox", "--headless", "--marionette", "--no-remote", "--profile", profile, "--window-size=1400,1000"],
    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


class Marionette:
    """The Marionette wire protocol: `length:json` frames; commands [0, id, name, params]."""

    def __init__(self, port):
        deadline = time.time() + 40
        while True:
            try:
                self.sock = socket.create_connection(("127.0.0.1", port), timeout=30)
                break
            except OSError:
                if time.time() > deadline:
                    raise
                time.sleep(0.25)
        self.buf, self.n = b"", 0
        self.read()  # the greeting

    def read(self):
        while b":" not in self.buf:
            self.buf += self.sock.recv(65536)
        size, rest = self.buf.split(b":", 1)
        size = int(size)
        while len(rest) < size:
            rest += self.sock.recv(65536)
        self.buf = rest[size:]
        return json.loads(rest[:size])

    def cmd(self, name, params=None):
        self.n += 1
        data = json.dumps([0, self.n, name, params or {}]).encode()
        self.sock.sendall(str(len(data)).encode() + b":" + data)
        while True:
            msg = self.read()
            if isinstance(msg, list) and msg[0] == 1 and msg[1] == self.n:
                if msg[2]:
                    raise RuntimeError(f"{name}: {msg[2]}")
                return msg[3]

    def js(self, script, *args):
        return self.cmd("WebDriver:ExecuteScript", {"script": script, "args": list(args)}).get("value")

    def wait(self, script, what, timeout=10):
        deadline = time.time() + timeout
        while time.time() < deadline:
            if self.js(script):
                return True
            time.sleep(0.2)
        raise RuntimeError("timed out waiting for " + what)

    def shot(self, path):
        png = self.cmd("WebDriver:TakeScreenshot", {"full": True})["value"]
        with open(path, "wb") as f:
            f.write(base64.b64decode(png))


results = []


def check(name, ok, detail=""):
    results.append(bool(ok))
    print(("ok    " if ok else "FAIL  ") + name + (f"   ({detail})" if detail else ""))


ROWS = "return [...document.querySelectorAll('#items tr')].map(r => r.cells[0].innerText.split('\\n')[0])"
HISTORY = ("return fetch('/api/history?item=' + arguments[0]).then(r => r.json())"
           ".then(h => h.cells.filter(c => c.field === 'answer'))")
EDIT = """
  const [item, pick, text] = arguments;
  if (pick) document.querySelector(`input[name="pick-${item}"][value="${pick}"]`).click();
  const box = document.querySelector(`[data-key="text-${item}"]`);
  box.value = text;
  box.dispatchEvent(new Event('input', { bubbles: true }));
"""
SAVE = """
  const row = [...document.querySelectorAll('#items tr')].find(r => r.cells[0].innerText.startsWith('B' + arguments[0]));
  [...row.querySelectorAll('button')].find(b => b.textContent === 'Save').click();
"""

try:
    m = Marionette(port)
    m.cmd("WebDriver:NewSession", {"capabilities": {}})
    m.cmd("WebDriver:Navigate", {"url": url})
    m.wait("return document.querySelectorAll('#items tr').length > 0", "the items to render")
    rows = m.js(ROWS)
    check("the table renders the items", len(rows) >= 3, ", ".join(rows))
    tree = m.js("return [...document.querySelectorAll('nav button.node')].map(b => b.innerText)")
    check("the tree renders projects, workspaces, sessions, agents", len(tree) >= 5, " | ".join(tree))
    entries = m.js("return document.querySelectorAll('#timeline li').length")
    check("the timeline renders", entries > 0, f"{entries} entries")
    text = m.js("return document.querySelector('#items').innerText")
    parsed = m.js("return [...document.querySelectorAll('#items b')].some(b => b.textContent === 'not bold')")
    check("board text stays text", "<b>not bold</b>" in text and not parsed,
          f"literal tag shown: {'<b>not bold</b>' in text}; parsed as markup: {parsed}")
    ctx = m.js("""return Object.fromEntries([...document.querySelectorAll('#items tr')].map(r =>
        [r.cells[0].innerText.split('\\n')[0], r.querySelector('.ctx') ? r.querySelector('.ctx').innerText : '']))""")
    check("each item shows its repo, worktree, branch and full path",
          "branch main" in ctx.get("B1", "") and "/alpha" in ctx.get("B1", "")
          and "worktree alpha-feature" in ctx.get("B4", "") and "branch feature" in ctx.get("B4", "")
          and "/alpha-feature" in ctx.get("B4", "") and "not in git" in ctx.get("B3", ""),
          json.dumps(ctx))
    m.shot(os.path.join(out, "page-1-loaded.png"))

    # The owner answers B1 through the page: a real browser POST.
    m.js(EDIT, 1, "b", "go with b")
    m.js(SAVE, 1)
    m.wait("return fetch('/api/history?item=1').then(r => r.json()).then(h => h.cells.some(c => c.field === 'answer'))",
           "the answer to land")
    answers = m.js(HISTORY, 1)
    check("the page's POST landed as the owner's answer, client page",
          len(answers) == 1 and answers[0]["client"] == "page" and answers[0]["value"] == {"pick": "b", "text": "go with b"},
          json.dumps([{k: a[k] for k in ("version", "value", "client")} for a in answers]))
    m.wait("return [...document.querySelectorAll('#items .chip')].some(c => c.textContent === 'answered')",
           "B1 to show answered")
    check("the status shows answered after the answer", True)

    # A stale save: B3 is edited here, then answered elsewhere, then saved here.
    m.js(EDIT, 3, None, "first draft")
    m.js("""return fetch('/api/answer', {method: 'POST',
             headers: {'Content-Type': 'application/json', 'X-Focus-Board': '1'},
             body: JSON.stringify({item: 3, answer: {text: 'from another tab'}, expect: 0})}).then(r => r.status)""")
    m.js(SAVE, 3)
    # Only the refusal says "Refused:"; the note that a draft moved says "changed elsewhere" too.
    m.wait("return document.querySelector('#items').innerText.includes('Refused:')", "the refusal to show")
    answers = m.js(HISTORY, 3)
    check("a stale save is refused and shown; the newer answer stays",
          [a["value"] for a in answers] == [{"text": "from another tab"}], json.dumps([a["value"] for a in answers]))
    B3_ANSWERED = """return [...document.querySelectorAll('#items tr')]
        .some(r => r.cells[0].innerText.startsWith('B3') && r.querySelector('.chip').textContent === 'answered')"""
    try:
        m.wait(B3_ANSWERED, "B3 to show answered", timeout=1.0)
        check("after a refusal the page shows the newer state at once, not at the next poll", True)
    except RuntimeError as e:
        check("after a refusal the page shows the newer state at once, not at the next poll", False, str(e))
    m.shot(os.path.join(out, "page-2-answered-and-refused.png"))

    under = m.js("""const copy = [...document.querySelectorAll('nav button.node')]
            .find(b => b.innerText.includes('(copy of Smith)'));
        const li = copy && copy.closest('ul').closest('li');
        return li ? li.querySelector('button.node').innerText : null;""")
    check("a copied session sits under the session it was copied from",
          bool(under) and under.startswith("Smith"), str(under))
    hashes = m.js("""return [...document.querySelectorAll('nav button.node')]
        .map(b => b.innerText.split(' ·')[0])
        .filter(t => /^session [0-9a-z-]{8}$/.test(t));""")
    check("no session shows as a hash", not hashes, ", ".join(hashes) or "none")

    # Full text search (the owner's ask): typing narrows the table, clearing brings it back.
    TYPE = """const q = document.getElementById('q'); q.value = arguments[0];
              q.dispatchEvent(new Event('input', { bubbles: true }));"""
    try:
        m.js(TYPE, "ship tod")
        m.wait("return document.querySelectorAll('#items tr').length === 1", "the search to narrow the table", timeout=5)
        check("searching narrows the table", m.js(ROWS) == ["B3"], ", ".join(m.js(ROWS)))
        m.js(TYPE, "")
        m.wait("return document.querySelectorAll('#items tr').length > 1", "the table to come back", timeout=5)
    except RuntimeError as e:
        check("searching narrows the table", False, str(e))

    # The review button (B22 "b"): a press shows when it was asked, and who has not seen it yet.
    try:
        m.js("document.getElementById('review').click()")
        m.wait("const p = document.getElementById('reviewed'); "
               "return !p.hidden && p.innerText.includes('Review asked')", "the review line", timeout=5)
        line = m.js("return document.getElementById('reviewed').innerText")
        check("the review button asks the sessions", "Not yet:" in line, line[:90])
    except RuntimeError as e:
        check("the review button asks the sessions", False, str(e))

    # The tree counts each node's items not closed (owner, 2026-11-22): the whole board's count is
    # the rows the table shows, with the status filter at "not closed" and no search.
    badge = m.js("return document.querySelector('nav button.node').getAttribute('data-count') || ''")
    rows = m.js(ROWS)
    check("the tree counts the items not closed", badge == str(len(rows)), f"count {badge}, rows {len(rows)}")

    # Ageing (owner, 2026-11-26, B45 "c"): each item not closed says on whom it waits, and for how
    # long; ordering by "waiting longest" keeps the same rows.
    ages = m.js("return [...document.querySelectorAll('#items .age')].map(a => a.innerText)")
    check("each waiting item says on whom", len(ages) == len(rows) and all(a.startswith("waiting on ") for a in ages),
          "; ".join(ages[:2]))
    ORDER = "const o = document.getElementById('order'); o.value = arguments[0]; o.dispatchEvent(new Event('change'))"
    m.js(ORDER, "waiting")
    reordered = m.js(ROWS)
    check("ordering by waiting keeps the rows", sorted(reordered) == sorted(rows), ", ".join(reordered))
    m.js(ORDER, "raised")

    labels = m.js("return [...document.querySelectorAll('nav button.node')].map(b => b.innerText.split(' ·')[0])")
    check("a project with one workspace has no workspace level; one with two has it",
          labels.count("beta") == 1 and labels.count("alpha") == 2 and "alpha-feature" in labels,
          " | ".join(labels))

    # A tree node narrows the table, even when it is picked while a refresh is running.
    try:
        m.js("""const page = window.wrappedJSObject || window;
                page.refresh();
                [...document.querySelectorAll('nav button.node')].find(b => b.innerText.startsWith('beta')).click();""")
        m.wait("return document.querySelectorAll('#items tr').length === 1", "the table to narrow", timeout=4)
        check("a node picked during a running refresh narrows the table", m.js(ROWS) == ["B3"], ", ".join(m.js(ROWS)))
    except RuntimeError as e:
        check("a node picked during a running refresh narrows the table", False, str(e))

    # Another page framing the board: its own writes would pass the guard, so it must not load.
    frame_page = os.path.join(out, "frame.html")
    with open(frame_page, "w") as f:
        f.write(f'<!doctype html><title>framer</title><iframe src="{url}" width="900" height="500"></iframe>')
    m.cmd("WebDriver:Navigate", {"url": "file://" + frame_page})
    time.sleep(2)
    try:
        m.cmd("WebDriver:SwitchToFrame", {"id": 0})
        framed = bool(m.js("return !!document.querySelector('#items')"))
    except RuntimeError:
        framed = False
    check("another page cannot frame the board", not framed, "framed" if framed else "refused")
except Exception as e:  # report, then clean up
    check("the run itself", False, repr(e))
finally:
    try:
        m.cmd("Marionette:Quit", {"flags": ["eForceQuit"]})
    except Exception:
        pass
    try:
        firefox.wait(timeout=15)
    except subprocess.TimeoutExpired:
        firefox.kill()
    shutil.rmtree(profile, ignore_errors=True)

print(f"{sum(results)}/{len(results)} checks passed")
sys.exit(0 if results and all(results) else 1)
