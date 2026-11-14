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

## `gateway.toml`

The gateway commands create this file once when it is absent (`gateway token` or `gateway up`) and
never rewrite it. Set `guild_id`, then restart the gateway to apply changes.

```toml
# A2AMX gateway settings. Restart the gateway to apply changes. a2amx writes this file once and never rewrites it.
# guild_id: Discord server ID (required to run the gateway).
# guild_id = "123456789012345678"
# control_category: Discord category for the control channel.
# control_category = "a2amx"
# control_channel: Discord channel used for control messages.
# control_channel = "control"
# max_members: Pause the gateway when the server exceeds this member count.
# max_members = 2
# sync: Interval between Discord synchronization checks.
# sync = "15s"
# idle: Interval between idle gateway checks.
# idle = "1s"
# member_check: Interval between member-count checks.
# member_check = "1m"
# reconnect: Delay before reconnecting after a gateway failure.
# reconnect = "5s"
```

| Key | Default | Rule |
| --- | --- | --- |
| `guild_id` | unset | 1–20 ASCII digits; required by `gateway up` |
| `control_category` | `a2amx` | 1–100 lowercase letters, digits, or `-` |
| `control_channel` | `control` | 1–100 lowercase letters, digits, or `-` |
| `max_members` | `2` | At least 1 |
| `sync` | `15s` | `1ms` through `3650d` |
| `idle` | `1s` | `1ms` through `3650d` |
| `member_check` | `1m` | `1ms` through `3650d` |
| `reconnect` | `5s` | `1ms` through `3650d` |

Durations are decimal digits followed by exactly one unit: `ms`, `s`, `m`, `h`, or `d`; `ms` is
matched before `s`. Gateway keys accept any value from `1ms` through `3650d`. Gateway intervals
may use milliseconds for short test runs. Daemon keys must be whole seconds: `1000ms` is accepted
but `1500ms` is rejected. No signs, spaces, fractions, compound values or bare numbers are accepted.
Unknown keys are rejected. `api_base` and `gateway_url` are accepted only as test overrides and
are intentionally hidden from this template and `config show`.

The gateway file is read strictly: it must be a regular owner-only (`0600`) file, not a symlink,
and no larger than 64 KiB. `a2amx config show` prints the resolved gateway values and their
`default` or `file` source without creating either configuration file.
