# shared spec

Everything in here is platform independent. The windows app reads these files
today, and a future mac port should be able to build against the same ones
without changes.

| file | what it is |
| --- | --- |
| `games.json` | `[{ "id": "<discord application id>", "name": "<game name>" }]`, sorted by name. Generated, never edited by hand. |
| `preset.schema.json` | JSON Schema for a saved preset and for the on-disk preset store. |
| `scripts/fetch-games.mjs` | Regenerates `games.json` from discord's public endpoint. |

## regenerating the game list

```sh
node shared/scripts/fetch-games.mjs
```

It downloads `https://discord.com/api/v9/applications/detectable` (about 13 MB),
keeps only `id` and `name`, drops duplicates and sorts by name. If the request
fails it exits non-zero and leaves the existing file alone. It never makes up
ids. Needs node 18+ (global `fetch`, no dependencies).

The app has a "refresh game list" button that does the same thing at runtime and
caches the result in its data dir. The bundled `games.json` is the fallback.

## the discord ipc protocol

Discord's desktop client listens on a local socket. Anything on the machine can
connect to it, say which application it is (a `client_id`), and then set a rich
presence for that application. There is no oauth for this, only presence
commands. This is the same thing every "game sdk" does under the hood.

### finding the socket

Try index `N` from 0 to 9 and use the first one that accepts a connection.
(Index 0 is the first discord client running, 1 is the second, and so on, which
is how stable, ptb and canary can coexist.)

| platform | endpoint |
| --- | --- |
| windows | named pipe `\\.\pipe\discord-ipc-N` |
| macos / linux | unix domain socket `$TMPDIR/discord-ipc-N` (`/tmp` if unset) |

The mac app must not run in the app sandbox, because the sandbox blocks that path.

### frames

Everything after connecting is a stream of frames:

```
+----------------+----------------+---------------------------+
| opcode (u32 LE)| length (u32 LE)| payload: length bytes of  |
|                |                | utf-8 json                |
+----------------+----------------+---------------------------+
```

The header is always 8 bytes, little endian on every platform. A frame can be
split across reads (or several frames can arrive in one read), so a reader has
to buffer until it has `8 + length` bytes.

| opcode | name | direction | payload |
| --- | --- | --- | --- |
| 0 | handshake | client to discord | `{"v":1,"client_id":"<id>"}` |
| 1 | frame | both | a command, response or event |
| 2 | close | both | `{"code":<n>,"message":"<text>"}` |
| 3 | ping | both | json, echoed back |
| 4 | pong | both | the json from the ping |

mimic rejects frames whose declared length is over 1 MiB, and rejects unknown
opcodes, rather than trying to allocate whatever the peer claims.

### handshake

1. Connect to the socket.
2. Send opcode 0 with `{"v":1,"client_id":"<application id>"}`.
3. Wait for an opcode 1 frame that looks like

   ```json
   {"cmd":"DISPATCH","evt":"READY","data":{"v":1,"config":{...},"user":{...}},"nonce":null}
   ```

   That is the only signal the connection is usable. Don't send commands before it.
4. If the application id is not valid, discord answers with an opcode 2 close
   frame (code 4000, "Invalid Client ID") instead of READY, or with an
   `"evt":"ERROR"` dispatch. Both mean "bad app id".

Close codes seen in practice (from the discord rpc docs; the exact set is
version dependent, so treat unknown codes as a generic failure):

| code | meaning |
| --- | --- |
| 1000 | closed normally |
| 4000 | invalid client id |
| 4001 | invalid origin |
| 4002 | rate limited |
| 4003 | token revoked |
| 4004 | invalid version |
| 4005 | invalid encoding |

### setting the activity

After READY, send an opcode 1 frame:

```json
{
  "cmd": "SET_ACTIVITY",
  "args": {
    "pid": 12345,
    "activity": {
      "details": "Ranked match",
      "state": "In queue",
      "timestamps": { "start": 1700000000 },
      "assets": {
        "large_image": "key", "large_text": "hover text",
        "small_image": "key", "small_text": "hover text"
      },
      "party": { "size": [2, 4] },
      "buttons": [{ "label": "Join", "url": "https://example.com" }]
    }
  },
  "nonce": "<uuid>"
}
```

- `pid` is the id of a real running process (mimic uses its own).
- `nonce` is echoed back in the response so replies can be matched to requests.
- To clear the presence, send the same command with `"activity": null`.
- Discord replies with a frame carrying the same `nonce`: `"evt": null` and the
  activity as `data` on success, or `"evt": "ERROR"` with
  `{"code":<n>,"message":"<text>"}` on failure.

Field rules the builder enforces so discord doesn't reject the payload:

- empty strings and empty objects are removed entirely, never sent as `""`
- `details`, `state`: at most 128 characters
- image keys: at most 256 characters, hover texts at most 128
- button `label`: 1 to 32 characters, `url`: `http(s)` only, at most 512 characters
- at most 2 buttons
- `party.size` is `[current, max]`, both at least 1, `current <= max`
- `timestamps.start` is unix seconds

Image keys only resolve to artwork if the application id you connected with
owns an asset with that name in its developer portal. For games from the
detectable list that is the game's publisher, not you, so custom keys will
usually show nothing. Whether a full url works as an image key over ipc is
**untested** here, so don't rely on it.

Buttons are only rendered for other people looking at your profile. You never
see them on your own.

### rate limits

Discord drops presence updates that arrive too quickly. mimic treats it as
about 1 update per 15 seconds: edits are coalesced, the most recent state is the
one sent, and the ui shows a countdown while a send is waiting.

### one client id per connection

A connection is bound to the `client_id` from its handshake. To show a
different game you close the connection and open a new one with the new id.
Never send a second handshake on the same socket.

### reconnecting

If the pipe closes or read/write fails (discord quit or restarted), drop the
connection, wait with exponential backoff (1s, 2s, 4s ... capped at 30s) and
walk pipes 0 to 9 again. After READY, re-send the last desired activity.

### reference test vectors

An empty `SET_ACTIVITY` clear frame for nonce `n` is the 8 byte header followed
by the json. For the handshake `{"v":1,"client_id":"1"}` (23 bytes) the full
frame is:

```
00 00 00 00  17 00 00 00  {"v":1,"client_id":"1"}
```
