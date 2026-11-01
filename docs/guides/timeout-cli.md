# Installing a timeout CLI

Stock macOS does not ship `timeout`. sysprims does not install an unprefixed
`timeout` binary on `PATH`.

## Options

### sysprims (this project)

```bash
sysprims timeout 30s -- cmd
```

This is the supported sysprims CLI form. There is no new unprefixed `timeout`
binary.

### uutils coreutils

Install [uutils/coreutils](https://github.com/uutils/coreutils) and use its
`timeout`. Like GNU coreutils 9.11, uutils `timeout` calls `setpgid` and
signals the process group by default; `--foreground` opts out.

### Homebrew GNU coreutils

```bash
brew install coreutils
```

The formula prefixes commands with `g`, so GNU timeout is `gtimeout`. Because
stock macOS has no `timeout`, the current Homebrew formula may also symlink an
unprefixed `timeout`. Confirm with `which timeout` and `timeout --version`.
You can also put Homebrew's `gnubin` directory on `PATH` for unprefixed names.

GNU coreutils is GPL-3.0; that path is a host CLI, not something to link into
an application.

## BusyBox

BusyBox `timeout` is **not** GNU-parity. Options, duration syntax, exit codes
(124–127 / 128+N), `-k` / `-p` / `-f`, TTY handling, and process-group
behavior differ. Do not treat it as a drop-in for GNU 9.11, uutils, or
`sysprims timeout`.
