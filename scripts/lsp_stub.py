#!/usr/bin/env python3
"""A language server that only pretends, so the client can be tested without installing one.

Speaks just enough of the protocol to exercise CleeCode's side of it: answers `initialize`,
publishes one warning and one error against whatever file it is told was opened, and answers
`textDocument/completion`, `textDocument/definition`, `textDocument/references`,
`textDocument/documentSymbol` and `textDocument/hover`. It echoes back the
URI it was given rather than inventing one, which
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


def define(message):
    """A definition built out of the question, like the completion answer above.

    The line it points at is the *word's own* line plus one, and the file it names is the file it
    was asked about. Both are read back off the screen after the jump, so an answer with a canned
    line number in it could not pass: the check is that the client went where it was told."""
    params = message.get("params", {})
    uri = params.get("textDocument", {}).get("uri", "")
    position = params.get("position", {})
    line = position.get("line", 0)
    character = position.get("character", 0)
    word = word_at(TEXTS.get(uri, ""), line, character)
    # One line further down than the cursor, so a client that ignored the answer and stayed put
    # would land on the wrong row and be caught.
    target = line + 1
    log(f"   defining {word!r} at line {target}")
    send({"jsonrpc": "2.0", "id": message.get("id"), "result": {
        "uri": uri,
        "range": {
            "start": {"line": target, "character": 0},
            "end": {"line": target, "character": max(1, len(word))},
        },
    }})


def references(message):
    """Two uses of the word, built out of the question like every other answer here.

    They are the two lines *after* the one asked about, in the file that was asked about. A
    client that listed them against the wrong file, or that read the rows off its own cursor
    instead of off the answer, lands somewhere the driver can see is wrong.

    The `context` is read rather than ignored: without `includeDeclaration` only one use comes
    back, so a client that dropped the one member this request carries beyond a position gets a
    list one row short of the one the checks are written against."""
    params = message.get("params", {})
    uri = params.get("textDocument", {}).get("uri", "")
    line = params.get("position", {}).get("line", 0)
    declared = params.get("context", {}).get("includeDeclaration")
    log(f"   listing the uses from line {line}, includeDeclaration={declared!r}")
    offsets = (1, 2) if declared else (1,)
    send({"jsonrpc": "2.0", "id": message.get("id"), "result": [
        {"uri": uri, "range": {"start": {"line": line + at, "character": 0},
                               "end": {"line": line + at, "character": 3}}}
        for at in offsets
    ]})


def symbols(message):
    """A small nested tree, named after the file it was asked about.

    `documentSymbol` carries no position for an answer to be built out of, so the names carry
    the document instead: a client that asked about the wrong file gets names that say so.

    Nested rather than flat on purpose. CleeCode does not tell a server it can read the nested
    shape and reads it anyway — servers send what they send — and this is the only place that
    claim is put to a real client end to end."""
    uri = message.get("params", {}).get("textDocument", {}).get("uri", "")
    stem = uri.rsplit("/", 1)[-1].rsplit(".", 1)[0]
    log(f"   listing the symbols of {stem}")

    def span(line):
        return {"start": {"line": line, "character": 0},
                "end": {"line": line, "character": 3}}

    send({"jsonrpc": "2.0", "id": message.get("id"), "result": [{
        "name": "outer_%s" % stem, "kind": 12,
        "range": span(0), "selectionRange": span(0),
        # On the last line of the fixture, which is neither where the cursor is nor where its
        # parent is: a client that jumped to the wrong row lands somewhere visibly else.
        "children": [{"name": "inner_%s" % stem, "kind": 6,
                      "range": span(3), "selectionRange": span(3)}],
    }]})


def hover(message):
    """A hover whose first line names the word and the place it was asked about.

    Wrapped in a code fence and followed by prose, because that is what a real server sends and
    because the client has to take exactly the first line out of it — a status bar that showed
    the fence, or the paragraph, would be showing markup."""
    params = message.get("params", {})
    uri = params.get("textDocument", {}).get("uri", "")
    position = params.get("position", {})
    line = position.get("line", 0)
    character = position.get("character", 0)
    word = word_at(TEXTS.get(uri, ""), line, character)
    log(f"   hovering {word!r}")
    send({"jsonrpc": "2.0", "id": message.get("id"), "result": {"contents": {
        "kind": "markdown",
        "value": f"```rust\nkind_of_{word}\n```\n\n---\n\nProse nobody has room for.",
    }}})


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
        elif method == "textDocument/definition":
            define(message)
        elif method == "textDocument/references":
            references(message)
        elif method == "textDocument/documentSymbol":
            symbols(message)
        elif method == "textDocument/hover":
            hover(message)
        elif method == "exit":
            return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (BrokenPipeError, KeyboardInterrupt):
        os._exit(0)
