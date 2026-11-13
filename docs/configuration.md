# Configuration

A2AMX stores operator configuration under the root directory (`~/.a2amx`, `A2AMX_HOME`, or
`--home`). The `config/` directory is owner-only; its layout is described in
[Architecture](architecture.md#directory-layout). Confined sessions see `config/` read-only.

## `daemon.toml`

The daemon creates this file once on a foreground or background start when it is absent. It
never rewrites an existing file. Restart the daemon to apply changes.

```toml
# A2AMX daemon policy. Restart the daemon to apply changes. The daemon writes this file once and never rewrites it.
# listen: addresses to bind.
# listen = ["127.0.0.1:0"]
# host_name: optional name shown in addresses.
# host_name = "host-a"
# owner_away: grace after interactive attachment ends.
# owner_away = "1m"
# [limits]
# max_pending_per_recipient: pending message cap.
# max_pending_per_recipient = 50
# rate_per_minute: agent send rate.
# rate_per_minute = 20
# human_inbound_per_minute: human inbound rate.
# human_inbound_per_minute = 20
# human_outbound_per_minute: human outbound rate.
# human_outbound_per_minute = 30
# max_stored_messages: stored message cap.
# max_stored_messages = 10000
# retention: message retention.
# retention = "7d"
# lease: remote lease duration.
# lease = "1d"
# code_ttl: pairing code lifetime.
# code_ttl = "10m"
```

### Keys and bounds

| Key | Default | Allowed values |
| --- | --- | --- |
| `listen` | `127.0.0.1:0` | One or more socket addresses |
| `host_name` | unset | Non-empty string when set |
| `owner_away` | `1m` | `0s` through `3650d` |
| `limits.max_pending_per_recipient` | `50` | `1` through `u32::MAX` |
| `limits.rate_per_minute` | `20` | `1` through `u32::MAX` |
| `limits.human_inbound_per_minute` | `20` | `1` through `u32::MAX` |
| `limits.human_outbound_per_minute` | `30` | `1` through `u32::MAX` |
| `limits.max_stored_messages` | `10000` | `1` through `u32::MAX` |
| `limits.retention` | `7d` | `1s` through `3650d` |
| `limits.lease` | `1d` | `1s` through `3650d` |
| `limits.code_ttl` | `10m` | `1s` through `3650d` |

Durations are decimal digits followed by exactly one unit: `s`, `m`, `h`, or `d`. No signs,
spaces, fractions, compound values or bare numbers are accepted. Values are resolved per key
with this precedence: command-line override, environment override, file, then Rust default.
`--listen` replaces the complete file list; `--host-name` and `--owner-away-secs` override their
file values. `A2AMX_OWNER_AWAY_SECS` is the environment override for `owner_away`.

`a2amx config show` prints each resolved value and its source without starting a daemon or
creating the file. Invalid configuration refuses daemon startup with the path and key in the
error. The file is operator policy: an unconfined operator can edit it; a symlink to a regular
file is followed as an operator-selected configuration path; confined sessions only see `config/`
read-only.
