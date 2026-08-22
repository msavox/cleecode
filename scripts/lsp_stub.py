#!/usr/bin/env python3
"""A language server that only pretends, so the client can be tested without installing one.

Speaks just enough of the protocol to exercise CleeCode's side of it: answers `initialize`,
publishes one warning and one error against whatever file it is told was opened, and answers
`textDocument/completion`. It echoes back the URI it was given rather than inventing one, which
is the whole point — a canned URI would pass even if the client's path-to-URI encoding were
broken, and that encoding is the part most likely to be.

The completion answer is built the same way, out of what was asked rather than out of a list
written here: it names the line and column it was asked about, and the word it found there in
the text it was last sent. A canned list would pass with the position arithmetic broken, with
the file never sent, or with the request going out against yesterday's text — which are three of
the four things that can actually go wrong on the client's side.

Used two ways: by the process-level test in src/lsp.rs, and by scripts/drive_lsp.py, which
installs it under the name CleeCode looks for and drives the real editor against it.

    CLEECODE_STUB_LINE=2   which buffer line to mark (default 1)
"""

import json
import os
import sys

LINE = int(os.environ.get("CLEECODE_STUB_LINE", "1"))
LOG = os.environ.get("CLEECODE_STUB_LOG")

# The text of each file as it was last sent to us, so a completion answer can be about the text
# the client says it has rather than about the file on disk.
TEXTS = {}


def log(text):
    """Where a stub can say what it was actually asked. Its stderr goes to /dev/null — a server
    that wrote to the terminal would draw over the editor — so there is nowhere else for it."""
    if LOG:
        with open(LOG, "a") as handle:
            handle.write(text + "\n")


def read_message(stream):
    length = None
    while True:
        line = stream.readline()
        if not line:
            return None
        line = line.decode("utf-8", "replace").strip()
        if not line:
            break
        if line.lower().startswith("content-length:"):
            length = int(line.split(":", 1)[1].strip())
    if length is None:
        return None
    return json.loads(stream.read(length).decode("utf-8", "replace"))


def send(payload):
    body = json.dumps(payload).encode("utf-8")
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body))
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()


def publish(uri):
    send({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": uri,
            "diagnostics": [
                {"range": {"start": {"line": LINE, "character": 8},
                           "end": {"line": LINE, "character": 13}},
                 "severity": 2, "message": "unused variable: `dummy`"},
                {"range": {"start": {"line": LINE + 1, "character": 4},
                           "end": {"line": LINE + 1, "character": 9}},
                 "severity": 1, "message": "cannot find value `nope` in this scope"},
            ],
        },
    })


def word_at(text, line, character):
    """The identifier ending at a position, in the text we were last sent."""
    lines = text.split("\n")
    if line >= len(lines):
        return ""
    head = lines[line][:character]
    word = ""
    for c in reversed(head):
        if c.isalnum() or c == "_":
            word = c + word
        else:
            break
    return word


def complete(message):
    """An answer assembled out of the question, so it cannot pass while the question is wrong."""
    params = message.get("params", {})
    uri = params.get("textDocument", {}).get("uri", "")
    position = params.get("position", {})
    line = position.get("line", 0)
    character = position.get("character", 0)
    word = word_at(TEXTS.get(uri, ""), line, character)
    log(f"   completing {word!r} at line {line} column {character}")
    send({"jsonrpc": "2.0", "id": message.get("id"), "result": {"isIncomplete": False, "items": [
        # No word at its head. Sorted first on purpose: if the client offered it, it would be the
        # top row of the popup, which is the loudest possible place for the failure to show.
        {"label": "&reference", "sortText": "0000"},
        # What was asked, said back. The name a passing screen has to show.
        {"label": f"{word}_line{line}_col{character}", "sortText": "0001"},
        # A label written to be read rather than typed: the brackets must not reach the buffer.
        {"label": f"{word}plicate(…)", "sortText": "0002"},
        # The same word twice, which is one row.
        {"label": f"{word}plicate()", "sortText": "0003"},
    ]}})


def main():
    while True:
        message = read_message(sys.stdin.buffer)
        if message is None:
            return 0
        method = message.get("method")
        log(f"<- {method or ('response ' + str(message.get('id')))}")
        if method == "initialize":
            send({"jsonrpc": "2.0", "id": message.get("id"), "result": {"capabilities": {
                # UTF-8, so the columns above mean characters and the test is about the client's
                # plumbing rather than about its UTF-16 arithmetic, which has its own tests.
                "positionEncoding": "utf-8",
                "textDocumentSync": 1,
            }}})
        elif method in ("textDocument/didOpen", "textDocument/didChange"):
            document = message["params"]["textDocument"]
            uri = document["uri"]
            if method == "textDocument/didOpen":
                TEXTS[uri] = document.get("text", "")
            else:
                TEXTS[uri] = message["params"]["contentChanges"][0].get("text", "")
            log(f"   publishing against {uri}")
            publish(uri)
        elif method == "textDocument/completion":
            complete(message)
        elif method == "exit":
            return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (BrokenPipeError, KeyboardInterrupt):
        os._exit(0)
