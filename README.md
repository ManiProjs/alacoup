# AlacoUp

**Install, update, and manage the Alaco compiler toolchain.** AlacoUp is a small native command-line tool written in Rust. It installs published Alaco releases, checks your environment, and forwards compiler commands to the installed compiler rather than reimplementing the language toolchain.

[Alaco language and compiler](https://github.com/ManiProjs/alaco) · [Alaco documentation](https://maniprojs.github.io/alaco/)

## Features

- Install the latest stable Alaco release or choose an exact version.
- Keep multiple compiler versions installed, switch between them, and remove inactive versions.
- Install published preview builds with `alacoup install --channel preview`.
- Use colored status messages and a download progress bar in interactive terminals (plain output remains available for pipes and `NO_COLOR`).
- Check whether a newer AlacoUp manager release is available with `alacoup self-update`.
- Verify downloaded release assets against the SHA-256 values in Alaco's release manifest.
- Keep compiler versions in separate versioned directories.
- Check the installation and common native build tools with `doctor`.
- Forward compiler commands while preserving argument boundaries and exit status.
- Bootstrap AlacoUp and Alaco together using the provided platform installers.
- Run formatting, tests, linting, and build checks in GitHub Actions on macOS, Linux, and Windows.

## Install AlacoUp and Alaco

The bootstrap installer downloads a prebuilt AlacoUp release into `~/.alacoup/bin`, adds a note if that directory is missing from `PATH`, and runs `alacoup install` to install Alaco and its standard library. Rust is not required to use the prebuilt installer.

### macOS and x86-64 Linux

Review [`install.sh`](install.sh), then run:

```sh
curl --fail --location --proto '=https' --tlsv1.2 \
  https://raw.githubusercontent.com/ManiProjs/alacoup/main/install.sh | sh
```

Supported macOS architectures are Apple Silicon (`arm64`) and Intel (`x86_64`). Linux currently supports x86-64 (`x86_64-unknown-linux-gnu`).

### Windows PowerShell

Review [`install.ps1`](install.ps1), then run:

```powershell
irm https://raw.githubusercontent.com/ManiProjs/alacoup/main/install.ps1 | iex
```

The Windows installer currently targets x86-64 Windows with the MSVC toolchain.

### Choose an AlacoUp release

Set `ALACOUP_VERSION` to a published tag before running the installer, for example `v0.1.0`. The default `latest` selects the latest published AlacoUp release. Alaco's compiler version is selected separately by the manager's release manifest; use `alacoup install <version>` to request an exact compiler version.

After installation, verify your environment:

```sh
alacoup --version
alacoup doctor
```

If your shell cannot find `alacoup`, add `~/.alacoup/bin` to `PATH` and open a new shell.

## Quick start

Install AlacoUp from source if you already have Rust, or use one of the prebuilt installers above. Then install Alaco and create a project:

```sh
cargo install --path .
alacoup install
alacoup doctor
alacoup new hello
cd hello
alacoup exec check
alacoup run
```

Install the compiler before running `doctor` on a fresh machine. If Alaco is already installed and you are troubleshooting it, run `alacoup doctor` first. The `new` command is forwarded to Alaco, so the project scaffold depends on the installed compiler version.

## Commands

| Command | Description |
| --- | --- |
| `alacoup --help` | Show manager commands and options. |
| `alacoup --version` | Print the AlacoUp version. |
| `alacoup check` | Check the latest stable compiler release and platform support. |
| `alacoup latest` | Print the latest stable Alaco version. |
| `alacoup self-update` | Check GitHub for a newer AlacoUp release. If one is available, print its release notes and installer instructions. |
| `alacoup upgrade` | Download, verify, and install the latest AlacoUp release. |
| `alacoup upgrade --check` | Check for a newer AlacoUp release without installing it. |
| `alacoup install [version]` | Install the latest release or a specific version, such as `0.1.0`. |
| `alacoup install --channel preview` | Install the latest published preview release. |
| `alacoup list` | List locally installed compiler versions; `*` marks the active version. |
| `alacoup list --available` | List published stable and preview releases. |
| `alacoup use <version>` | Switch the active compiler and its bundled standard library to an installed version. |
| `alacoup uninstall <version>` | Remove an installed version that is not active. |
| `alacoup install --binary-only` | Install/update the compiler binary without replacing the standard library. |
| `alacoup doctor` | Check the installed compiler, standard library, and common native tools. |
| `alacoup paths` | Print manager and standard-library paths. |
| `alacoup exec <command> [args…]` | Forward a command to Alaco, including `check`. |
| `alacoup <compiler-command> [args…]` | Forward other compiler commands directly. |

Forwarded commands depend on the installed compiler version and may include `new`, `init`, `explain`, `build`, `run`, `transpile`, `test`, `clean`, `manifest`, and `lsp`.

**Note:** `alacoup check` checks the manager's release/platform support. To check Alaco source code, use `alacoup exec check`. For example:

```sh
alacoup exec check
alacoup build src/main.aco --output build/app
alacoup run
```

The compiler's `test` command expects `.aco` test programs under `tests/`, or a test file/directory passed explicitly. It exits unsuccessfully if no tests are discovered.

## Installation paths and configuration

- AlacoUp and its compiler launcher: `~/.alacoup/bin/`
- Versioned compiler payloads: `~/.alacoup/versions/`
- Alaco standard library: `~/.alaco/stdlib/std/`

Environment variables:

| Variable | Purpose |
| --- | --- |
| `ALACOUP_HOME` | Override AlacoUp's installation root. |
| `ALACO_STDLIB_DIR` | Override the standard-library root. |
| `ALACOUP_VERSION` | Select a specific AlacoUp release in the bootstrap installers. |

Native compilation requires a compatible C++ compiler. `clang++` is recommended on macOS and Linux. `doctor` checks common prerequisites, but cannot guarantee that every local compiler configuration is compatible.

## Updates and recovery

AlacoUp downloads release assets over HTTPS, follows only permitted HTTPS redirects, limits download/archive sizes, validates archive entries, and checks asset SHA-256 against Alaco's published release manifest before installing them. The manifest is hosted at:

`https://github.com/ManiProjs/alaco/releases/latest/download/alacoup-manifest.json`

Compiler payloads are staged in a new version directory before activation. Standard-library replacement uses a backup, and if launcher activation fails, AlacoUp attempts to restore the previous standard library and keep the previous compiler launcher active. The `--binary-only` option leaves the standard library untouched.

These steps improve recovery from ordinary installation errors, but **the multi-path update is not fully crash-atomic**: sudden power loss or filesystem failure during activation can still leave compiler and standard-library state mismatched. The release manifest's checksum detects asset mismatches but does not independently authenticate the manifest itself. Keep backups for critical environments.

## Development

Requirements: a stable Rust toolchain with Cargo. Run these commands from the repository root:

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo check --locked --all-targets
cargo build --locked --release
```

The GitHub Actions CI workflow runs these checks and CLI smoke tests on macOS, Linux, and Windows for pushes and pull requests. Release tags matching `v*.*.*` trigger native builds for Apple Silicon macOS, Intel macOS, x86-64 Linux, and x86-64 Windows, then attach the archives to a GitHub Release.

## Project scope

AlacoUp manages the compiler installation and dispatches commands to Alaco. It does not implement a separate project manifest, package registry, dependency resolver, language server, or compiler backend. Language and project capabilities are defined by the installed Alaco release; see the [Alaco repository](https://github.com/ManiProjs/alaco) and [documentation](https://maniprojs.github.io/alaco/) for details.

## License

AlacoUp is distributed under the MIT license. See [`LICENSE`](LICENSE).
