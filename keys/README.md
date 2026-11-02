# Image signing keys

Atlas says who signed an image before it installs or flashes it. A signed
image has a minisign signature next to it, `<file>.minisig` (Gaia writes one
for every published output it signs). Atlas checks it against the keys it
trusts and names the key's owner:

```
helios-2026.2.0-dev.900-styx-ab12cd34.img.xz: signed by Prometheus Dynamics dev/example key (Prometheus Dynamics, dev/example)
warning: signed with a dev/example key (Prometheus Dynamics dev/example key): fine for testing, not a production release
```

Nothing is refused on a signature yet. An unsigned file, a key Atlas doesn't
trust, an expired key or a signature that doesn't match is a warning, until
a policy is chosen.

`atlas keys` lists the trusted keys and their owners; `atlas keys check
<file>` says who signed a file (exit 1 when Atlas can't vouch for it).

## The trusted keys

[`trusted-keys.json`](trusted-keys.json) is built into Atlas. Each entry is
one key and who owns it:

| Field | |
|---|---|
| `name` | What Atlas calls the key |
| `kind` | `minisign` (the only kind so far) |
| `key_id` | minisign's key id, 16 hex digits, as `minisign -G` prints it |
| `public_key` | The second line of the minisign `.pub` file |
| `owner` | A person or a company |
| `contact` | An address or URL to reach the owner |
| `purpose` | `dev` (dev and example builds; the private key isn't guarded) or `production` |
| `created`, `expires` | `YYYY-MM-DD`; `expires` may be `null` |
| `note` | Anything else worth knowing |

A new key (the production key, a third party's) is a new entry; no code
changes. Your own keys go in a file of the same shape, read on top of the
built-in ones: `$ATLAS_TRUSTED_KEYS`, else `trusted-keys.json` in Atlas's
config directory (`~/.config/atlas/` on Linux; `atlas keys` prints the path).

## The dev/example key

`minisign:1AB01E429EDCEAA6`, owner Prometheus Dynamics, purpose dev,
2026-11-02 to 2027-11-02. It signs dev builds of HeliOS and PhotonVision so
the signing path is exercised end to end. It is **not for production**: its
private key is unencrypted (minisign `-W`) on developer machines, so a
signature by it says only that a dev build came from one of them. The
production key comes later and is the user's.

Only the public key is in this repo. The private key lives outside any repo:

```
~/.config/atlas/dev-signing/minisign.key   (mode 0600)
~/.config/atlas/dev-signing/minisign.pub
```

Gaia signs a build with it when `ATLAS_DEV_SIGNING_KEY` holds the private
key's path (Gaia's `[image.signing] key = "env:ATLAS_DEV_SIGNING_KEY"`,
`optional = true`). Without the variable a dev build finishes unsigned with
a warning; Gaia doesn't fall back to the home directory, so a CI or someone
else's machine never signs with whatever key happens to be there.

```
export ATLAS_DEV_SIGNING_KEY=~/.config/atlas/dev-signing/minisign.key
```

On a machine without the key, copy it from one that has it (it's a dev key;
any developer may hold it), or make a new pair and add its public key here
as another dev entry:

```
minisign -G -W -s ~/.config/atlas/dev-signing/minisign.key \
  -p ~/.config/atlas/dev-signing/minisign.pub \
  -c "<owner> dev/example key - not for production"
```

To sign a file by hand: `minisign -S -s "$ATLAS_DEV_SIGNING_KEY" -m <file>`.
